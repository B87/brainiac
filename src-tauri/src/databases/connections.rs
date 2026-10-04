//! Saved connections (SPEC.md, Databases: Connections): rows in the core
//! database, each with where its password comes from (SPEC.md, Secrets), and
//! Test Connection.
//!
//! A password is never stored here. The Keychain item `brainiac/db:<id>` is
//! written only on a save, after a non-secret marker in the row says a write
//! is under way, so a save cut off between the two leaves a row that is
//! blocked rather than one that looks usable with an unknown password.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::driver::{Session, Target};
use super::postgres::PgTarget;
use crate::credentials::{
    describe, Binding, CredentialService, LeaseHandle, OwnerGate, Probe, Resolution, Secret,
    SecretBytes,
};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AppError, AppResult, CredentialOwner, CredentialPending, CredentialState,
    CredentialTest, DbAccess, DbConnection, DbEnvironment, DbKind, DbTestResult, DbTls,
    DbUrlFields, ErrorCode, RunsOn, SaveDbConnectionRequest, SecretEntry, SecretSource,
};

const DEFAULT_PORT: u16 = 5432;
const MAX_TIMEOUT_SECONDS: u32 = 3600;

/// The owner key of a connection's credential, which is also its Keychain account.
pub fn keychain_account(id: &str) -> String {
    format!("db:{id}")
}

fn binding(connection: &DbConnection) -> Binding {
    Binding {
        owner: keychain_account(&connection.id),
        source: connection.password.clone(),
        revision: connection.credential.revision,
        approved: !connection.credential.needs_approval,
        pending: connection.credential.pending,
        expires_at: None,
    }
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "This connection was changed since the form opened. Close the form and open it again.",
    )
}

/// A save cut off after its marker: the row stays blocked until saved again.
fn partial_save(e: AppError) -> AppError {
    AppError::new(
        e.code,
        format!(
            "{} The connection cannot be used until it is saved again.",
            e.message
        ),
    )
}

pub struct ConnectionService {
    db: Db,
    // `Arc` because accounts and connections share one credentials layer.
    credentials: Arc<CredentialService>,
}

impl ConnectionService {
    pub fn new(db: Db, credentials: Arc<CredentialService>) -> Self {
        ConnectionService { db, credentials }
    }

    fn complete(&self, mut connection: DbConnection) -> DbConnection {
        connection.password_ready = match connection.password {
            SecretSource::Ask => self.credentials.typed_ready(
                &keychain_account(&connection.id),
                connection.credential.revision,
            ),
            _ => true,
        };
        connection.file_size = connection
            .file_path
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        connection
    }

    /// Every connection, except those being removed.
    pub async fn list(&self) -> AppResult<Vec<DbConnection>> {
        let rows = self.db.call(|conn| list(conn)).await?;
        Ok(rows
            .into_iter()
            .filter(|c| c.credential.pending != Some(CredentialPending::Removal))
            .map(|c| self.complete(c))
            .collect())
    }

    pub async fn get(&self, id: &str) -> AppResult<DbConnection> {
        self.row(id)
            .await?
            .ok_or_else(|| AppError::not_found("That connection no longer exists."))
    }

    async fn row(&self, id: &str) -> AppResult<Option<DbConnection>> {
        let key = id.to_string();
        let row = self.db.call(move |conn| get(conn, &key)).await?;
        Ok(row.map(|c| self.complete(c)))
    }

    /// Create or update a connection (docs/architecture.md, Credentials:
    /// saving). A typed password goes to the Keychain, or for Ask is kept
    /// for this run; it is never stored here.
    pub async fn save(&self, request: SaveDbConnectionRequest) -> AppResult<DbConnection> {
        let fields = validate(&request)?;
        let typed = match fields.password {
            SecretSource::Store | SecretSource::Ask => {
                request.password.as_deref().map(Secret::new).transpose()?
            }
            _ => None,
        };
        let id = request
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let gate = self.credentials.gate(&keychain_account(&id)).await;
        let stored = match &request.id {
            Some(id) => Some(self.get(id).await?),
            None => None,
        };
        if let Some(s) = &stored {
            if request.expected_version.is_some_and(|v| v != s.version) {
                return Err(conflict());
            }
            if s.credential.pending == Some(CredentialPending::Removal) {
                return Err(AppError::validation(
                    "This connection is being removed. Retry the removal in Settings → Secrets.",
                ));
            }
        }
        let old_source = stored.as_ref().map(|s| s.password.clone());
        let old_revision = stored.as_ref().map_or(0, |s| s.credential.revision);
        let old_pending = stored.as_ref().and_then(|s| s.credential.pending);
        let was_unapproved = stored.as_ref().is_some_and(|s| s.credential.needs_approval);
        let destination_changed = stored
            .as_ref()
            .is_none_or(|s| !same_destination(&Fields::from(s), &fields));
        let source_changed = old_source.as_ref() != Some(&fields.password);
        let existing = stored.is_some();
        let expected = request.expected_version;

        match (&fields.password, typed) {
            // A new password for the Keychain: marker, item, then the row.
            (SecretSource::Store, Some(secret)) => {
                let marked = {
                    let (row_id, f, now) = (id.clone(), fields.clone(), now_rfc3339());
                    self.db
                        .call(move |conn| {
                            if existing {
                                mark_pending(conn, &row_id, CredentialPending::Save, expected)
                            } else {
                                insert_provisional(conn, &row_id, &f, &now)
                            }
                        })
                        .await
                };
                marked?;
                self.credentials.begin(&gate);
                let bytes = SecretBytes::from_text(secret.expose());
                self.credentials
                    .write_item(&gate, bytes)
                    .await
                    .map_err(partial_save)?;
                let revision = old_revision + 1;
                self.commit(&id, &fields, revision, None, existing)
                    .await
                    .map_err(partial_save)?;
                self.credentials.commit(&gate, revision);
                self.credentials.prime(
                    &gate,
                    revision,
                    SecretBytes::from_text(secret.expose()),
                    None,
                );
            }
            // Keep the item already in the Keychain.
            (SecretSource::Store, None) => {
                if old_source != Some(SecretSource::Store)
                    || old_pending == Some(CredentialPending::Save)
                {
                    return Err(AppError::validation(
                        "Enter the password to keep in the Keychain.",
                    ));
                }
                let revision = old_revision + i64::from(destination_changed);
                self.commit(&id, &fields, revision, None, existing).await?;
                if destination_changed || was_unapproved {
                    self.credentials.commit(&gate, revision);
                }
            }
            // Ask, None, or a source Brainiac only reads: the row first, then
            // the old Keychain item is deleted, never used as a fallback.
            (source, typed) => {
                let cleanup = old_source == Some(SecretSource::Store)
                    || matches!(
                        old_pending,
                        Some(CredentialPending::Save | CredentialPending::Cleanup)
                    );
                let interrupted = old_pending == Some(CredentialPending::Save);
                let changed = source_changed || destination_changed || interrupted;
                let revision = old_revision + i64::from(changed);
                let pending = cleanup.then_some(CredentialPending::Cleanup);
                self.commit(&id, &fields, revision, pending, existing)
                    .await?;
                if changed || was_unapproved {
                    self.credentials.commit(&gate, revision);
                }
                if let (SecretSource::Ask, Some(secret)) = (source, typed) {
                    let saved = self.get(&id).await?;
                    self.credentials
                        .supply(&binding(&saved), SecretBytes::from_text(secret.expose()))?;
                }
                if cleanup {
                    self.cleanup(&gate, &id).await;
                }
            }
        }
        drop(gate);
        let connection = self.get(&id).await?;
        tracing::info!(connection = %connection.id, kind = connection.kind.as_str(), "database connection saved");
        Ok(connection)
    }

    async fn commit(
        &self,
        id: &str,
        fields: &Fields,
        revision: i64,
        pending: Option<CredentialPending>,
        existing: bool,
    ) -> AppResult<()> {
        let (row_id, f, now) = (id.to_string(), fields.clone(), now_rfc3339());
        self.db
            .call(move |conn| upsert(conn, &row_id, &f, revision, pending, existing, &now))
            .await
    }

    /// Delete the old Keychain item after a move to another source. A
    /// failure leaves the cleanup pending, shown in Settings → Secrets.
    async fn cleanup(&self, gate: &OwnerGate, id: &str) {
        match self.credentials.delete_item(gate).await {
            Ok(()) => {
                let key = id.to_string();
                let cleared = self
                    .db
                    .call(move |conn| clear_pending(conn, &key, CredentialPending::Cleanup))
                    .await;
                if let Err(e) = cleared {
                    tracing::warn!(error = %e, "could not record a finished cleanup");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, connection = %id, "the old Keychain item could not be deleted")
            }
        }
    }

    /// Delete a connection: marked as being removed first, then its Keychain
    /// item (whatever its source is now), then the row. A failed deletion
    /// leaves the removal pending, for Settings → Secrets to retry.
    pub async fn delete(&self, id: &str, expected_version: i64) -> AppResult<()> {
        let gate = self.credentials.gate(&keychain_account(id)).await;
        let stored = self.get(id).await?;
        if stored.version != expected_version {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This connection was changed since it was shown. Look at it again before deleting it.",
            ));
        }
        let key = id.to_string();
        self.db
            .call(move |conn| {
                mark_pending(
                    conn,
                    &key,
                    CredentialPending::Removal,
                    Some(expected_version),
                )
            })
            .await?;
        self.credentials.begin(&gate);
        self.finish_removal(&gate, id).await?;
        tracing::info!(connection = %id, "database connection deleted");
        Ok(())
    }

    async fn finish_removal(&self, gate: &OwnerGate, id: &str) -> AppResult<()> {
        self.credentials.delete_item(gate).await.map_err(|e| {
            AppError::new(
                e.code,
                format!(
                    "The connection is no longer used, but its Keychain item could not be deleted: {} Retry in Settings → Secrets.",
                    e.message
                ),
            )
        })?;
        let key = id.to_string();
        self.db.call(move |conn| delete(conn, &key)).await?;
        self.credentials.forget(gate);
        Ok(())
    }

    /// Whether the connection is gone or being removed: its sessions close.
    pub async fn is_removed(&self, id: &str) -> bool {
        match self.row(id).await {
            Ok(Some(c)) => c.credential.pending == Some(CredentialPending::Removal),
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// Settings → Secrets, **Retry**: finish a pending cleanup or removal.
    pub async fn retry_cleanup(&self, id: &str) -> AppResult<()> {
        let gate = self.credentials.gate(&keychain_account(id)).await;
        let Some(stored) = self.row(id).await? else {
            return Ok(());
        };
        match stored.credential.pending {
            Some(CredentialPending::Cleanup) => {
                // A move back to the Keychain cleared the marker in its own save.
                if stored.password != SecretSource::Store {
                    self.credentials.delete_item(&gate).await?;
                }
                let key = id.to_string();
                self.db
                    .call(move |conn| clear_pending(conn, &key, CredentialPending::Cleanup))
                    .await
            }
            Some(CredentialPending::Removal) => self.finish_removal(&gate, id).await,
            Some(CredentialPending::Save) => Err(AppError::validation(
                "Edit the connection and save it again: enter the password, or choose another source.",
            )),
            None => Ok(()),
        }
    }

    /// Settings → Secrets, **Allow This Source**: confirm a restored source,
    /// for the revision the user was shown.
    pub async fn approve(&self, id: &str, revision: i64) -> AppResult<DbConnection> {
        let gate = self.credentials.gate(&keychain_account(id)).await;
        let key = id.to_string();
        self.db
            .call(move |conn| approve(conn, &key, revision))
            .await?;
        self.credentials.commit(&gate, revision);
        drop(gate);
        self.get(id).await
    }

    /// Refresh credential: the next use reads the source, or asks, again.
    pub fn refresh(&self, id: &str) {
        self.credentials.invalidate(&keychain_account(id));
    }

    /// Link a connection to a repository, or unlink it: the repository's
    /// side panel lists its connections.
    pub async fn link(
        &self,
        id: &str,
        repository_id: &str,
        linked: bool,
    ) -> AppResult<DbConnection> {
        let (key, repository) = (id.to_string(), repository_id.to_string());
        self.db
            .call(move |conn| set_link(conn, &key, &repository, linked))
            .await?;
        self.get(id).await
    }

    /// Keep the password of an `Ask` connection for this run, for the
    /// version of the connection it was asked for.
    pub async fn unlock(
        &self,
        id: &str,
        password: &str,
        expected_version: i64,
    ) -> AppResult<DbConnection> {
        let connection = self.get(id).await?;
        if connection.version != expected_version {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This connection was changed since its password was asked for. Try again.",
            ));
        }
        let secret = Secret::new(password)?;
        self.credentials.supply(
            &binding(&connection),
            SecretBytes::from_text(secret.expose()),
        )?;
        Ok(self.complete(connection))
    }

    /// Test Connection: connect once with the form's fields, reading the
    /// form's password source afresh, without using or changing what is
    /// kept for this run.
    pub async fn test(&self, request: SaveDbConnectionRequest) -> AppResult<DbTestResult> {
        let fields = validate(&request)?;
        let stored = match &request.id {
            Some(id) => self.row(id).await?,
            None => None,
        };
        let owner = keychain_account(request.id.as_deref().unwrap_or("draft"));
        let typed = match fields.password {
            SecretSource::Store | SecretSource::Ask => {
                request.password.as_deref().map(Secret::new).transpose()?
            }
            _ => None,
        };
        // Testing exactly what is saved is a test of the saved binding.
        let saved = stored.as_ref().filter(|s| {
            typed.is_none()
                && s.password == fields.password
                && same_destination(&Fields::from(*s), &fields)
        });
        if let Some(s) = saved {
            if s.credential.needs_approval {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "This connection's password source came from a restored backup. Allow it in Settings → Secrets before testing it.",
                ));
            }
        }
        let result = async {
            let password = match (&fields.password, typed) {
                (SecretSource::None, _) => None,
                (_, Some(typed)) => Some(typed),
                (SecretSource::Ask, None) => {
                    return Err(AppError::validation(
                        "Enter the password to test the connection.",
                    ))
                }
                (SecretSource::Store, None) => {
                    let readable = stored.as_ref().is_some_and(|s| {
                        s.password == SecretSource::Store
                            && s.credential.pending != Some(CredentialPending::Save)
                            && !s.credential.needs_approval
                    });
                    if !readable {
                        return Err(AppError::validation(
                            "Enter the password to keep in the Keychain.",
                        ));
                    }
                    self.probe(&owner, &fields.password).await?
                }
                (source, None) => self.probe(&owner, source).await?,
            };
            let session = Session::open(&target(&fields, password)).await?;
            Ok(DbTestResult {
                server_version: session.server_version().to_string(),
            })
        }
        .await;
        if let Some(s) = saved {
            self.credentials.record_test(
                &owner,
                s.credential.revision,
                CredentialTest {
                    at: now_rfc3339(),
                    ok: result.is_ok(),
                    message: match &result {
                        Ok(r) => format!("Connected: {}.", r.server_version),
                        Err(e) => e.message.clone(),
                    },
                },
            );
        }
        result
    }

    async fn probe(&self, owner: &str, source: &SecretSource) -> AppResult<Option<Secret>> {
        match self.credentials.probe(owner, source).await? {
            Probe::Secret(bytes) => Ok(Some(Secret::from_bytes(&bytes, &describe(source, owner))?)),
            Probe::NoCredential => Ok(None),
            Probe::InputRequired => Err(AppError::validation(
                "Enter the password to test the connection.",
            )),
        }
    }

    /// What a session for this connection connects to, and the lease of the
    /// password it uses: a server's refusal rejects exactly that lease.
    pub async fn target(
        &self,
        connection: &DbConnection,
    ) -> AppResult<(Target, Option<LeaseHandle>)> {
        let fields = Fields::from(connection);
        let binding = binding(connection);
        let resolved = self.credentials.resolve(&binding).await.map_err(|e| {
            if e.code == ErrorCode::NotFound && connection.password == SecretSource::Store {
                AppError::new(
                    ErrorCode::PermissionDenied,
                    "The connection's password is no longer in the Keychain. Edit the connection and enter it again.",
                )
            } else {
                e
            }
        })?;
        let (password, lease) = match resolved {
            Resolution::NoCredential => (None, None),
            Resolution::InputRequired => {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "Enter the connection's password to connect.",
                ))
            }
            Resolution::Secret(lease) => {
                let handle = LeaseHandle::new(Arc::clone(&self.credentials), lease);
                let source = describe(&connection.password, &binding.owner);
                match Secret::from_bytes(handle.lease().bytes(), &source) {
                    Ok(secret) => (Some(secret), Some(handle)),
                    Err(e) => {
                        handle.reject();
                        return Err(e);
                    }
                }
            }
        };
        Ok((target(&fields, password), lease))
    }

    /// Open a session for this connection. A password the server refuses
    /// is forgotten, so it is read or asked for again next time.
    pub async fn open(&self, connection: &DbConnection) -> AppResult<Session> {
        let (target, lease) = self.target(connection).await?;
        match Session::open(&target).await {
            Ok(session) => Ok(session),
            Err(e) => {
                if e.code == ErrorCode::Unauthenticated {
                    if let Some(lease) = &lease {
                        lease.reject();
                    }
                }
                Err(e)
            }
        }
    }

    /// Settings → Secrets: every connection with a password. Reads no secret.
    pub async fn secret_entries(&self) -> AppResult<Vec<SecretEntry>> {
        let rows = self.db.call(|conn| list(conn)).await?;
        Ok(rows
            .into_iter()
            .map(|c| self.complete(c))
            .filter(|c| c.password != SecretSource::None || c.credential.pending.is_some())
            .map(|c| {
                let owner = keychain_account(&c.id);
                SecretEntry {
                    owner: CredentialOwner::DbConnection { id: c.id.clone() },
                    label: c.name.clone(),
                    destination: destination(&c),
                    input_required: c.password == SecretSource::Ask && !c.password_ready,
                    last_test: self.credentials.last_test(&owner, c.credential.revision),
                    source: c.password,
                    state: c.credential,
                }
            })
            .collect())
    }
}

/// Where a connection's password is sent, in words.
fn destination(c: &DbConnection) -> String {
    match c.kind {
        DbKind::Sqlite => c.file_path.clone().unwrap_or_default(),
        DbKind::Postgres => format!(
            "{}:{}/{} as {}",
            c.host.as_deref().unwrap_or_default(),
            c.port.unwrap_or(DEFAULT_PORT),
            c.database.as_deref().unwrap_or_default(),
            c.user.as_deref().unwrap_or_default()
        ),
    }
}

/// Whether two sets of fields send the password to the same place, with
/// the same TLS policy. A name, an environment, or a time limit does not count.
fn same_destination(a: &Fields, b: &Fields) -> bool {
    a.kind == b.kind
        && a.file_path == b.file_path
        && a.host == b.host
        && a.port == b.port
        && a.database == b.database
        && a.user == b.user
        && a.tls == b.tls
        && a.ca_file == b.ca_file
}

/// A connection's fields, checked and normalized for its kind.
#[derive(Debug, Clone)]
struct Fields {
    name: String,
    kind: DbKind,
    environment: DbEnvironment,
    access: DbAccess,
    file_path: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    database: Option<String>,
    user: Option<String>,
    tls: Option<DbTls>,
    ca_file: Option<String>,
    password: SecretSource,
    timeout: u32,
    runs_on: Option<RunsOn>,
}

impl From<&DbConnection> for Fields {
    fn from(c: &DbConnection) -> Self {
        Fields {
            name: c.name.clone(),
            kind: c.kind,
            environment: c.environment,
            access: c.access,
            file_path: c.file_path.clone(),
            host: c.host.clone(),
            port: c.port,
            database: c.database.clone(),
            user: c.user.clone(),
            tls: c.tls,
            ca_file: c.ca_file.clone(),
            password: c.password.clone(),
            timeout: c.statement_timeout_seconds,
            runs_on: c.runs_on.clone(),
        }
    }
}

fn target(fields: &Fields, password: Option<Secret>) -> Target {
    let timeout = Duration::from_secs(fields.timeout.into());
    match fields.kind {
        DbKind::Sqlite => Target::Sqlite {
            path: PathBuf::from(fields.file_path.clone().unwrap_or_default()),
            timeout,
            writable: fields.access == DbAccess::ReadWrite,
        },
        DbKind::Postgres => Target::Postgres(PgTarget {
            host: fields.host.clone().unwrap_or_default(),
            port: fields.port.unwrap_or(DEFAULT_PORT),
            database: fields.database.clone().unwrap_or_default(),
            user: fields.user.clone().unwrap_or_default(),
            password,
            tls: fields.tls.unwrap_or(DbTls::Verify),
            ca_file: fields.ca_file.as_ref().map(PathBuf::from),
            statement_timeout: timeout,
            application_name: "Brainiac".to_string(),
        }),
    }
}

fn trimmed(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn validate(request: &SaveDbConnectionRequest) -> AppResult<Fields> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("Name the connection."));
    }
    if name.chars().count() > 100 {
        return Err(AppError::validation(
            "A connection's name is at most 100 characters.",
        ));
    }
    if request.statement_timeout_seconds == 0
        || request.statement_timeout_seconds > MAX_TIMEOUT_SECONDS
    {
        return Err(AppError::validation(format!(
            "The time limit is between 1 and {MAX_TIMEOUT_SECONDS} seconds."
        )));
    }
    let mut fields = Fields {
        name,
        kind: request.kind,
        environment: request.environment,
        access: request.access,
        file_path: None,
        host: None,
        port: None,
        database: None,
        user: None,
        tls: None,
        ca_file: None,
        password: SecretSource::None,
        timeout: request.statement_timeout_seconds,
        runs_on: None,
    };
    match request.kind {
        // SQLite has no password: like the server fields, a source is ignored.
        DbKind::Sqlite => {
            let path = trimmed(&request.file_path)
                .ok_or_else(|| AppError::validation("Choose the database file."))?;
            if !Path::new(&path).is_absolute() {
                return Err(AppError::validation(
                    "Choose the database file with its full path.",
                ));
            }
            if !Path::new(&path).is_file() {
                return Err(AppError::not_found(format!("There is no file at {path}.")));
            }
            if request.access == DbAccess::ReadWrite
                && super::sqlite::is_brainiac_file(Path::new(&path))
            {
                return Err(AppError::validation(
                    "This is one of Brainiac's own databases. It opens read only: writing to it behind Brainiac's back would break its data.",
                ));
            }
            fields.file_path = Some(path);
        }
        DbKind::Postgres => {
            let host =
                trimmed(&request.host).ok_or_else(|| AppError::validation("Enter the host."))?;
            if host.chars().any(char::is_whitespace) {
                return Err(AppError::validation("A host name has no spaces."));
            }
            fields.host = Some(host);
            fields.port = Some(match request.port {
                Some(0) => return Err(AppError::validation("The port is between 1 and 65535.")),
                Some(p) => p,
                None => DEFAULT_PORT,
            });
            fields.database = Some(
                trimmed(&request.database)
                    .ok_or_else(|| AppError::validation("Enter the database name."))?,
            );
            fields.user = Some(
                trimmed(&request.user).ok_or_else(|| AppError::validation("Enter the user."))?,
            );
            fields.tls = Some(request.tls.unwrap_or(DbTls::Verify));
            if let Some(ca) = trimmed(&request.ca_file) {
                if fields.tls != Some(DbTls::Verify) {
                    return Err(AppError::validation(
                        "A CA file is used only when TLS verifies the certificate.",
                    ));
                }
                if !Path::new(&ca).is_file() {
                    return Err(AppError::not_found(format!("There is no CA file at {ca}.")));
                }
                fields.ca_file = Some(ca);
            }
            fields.password = crate::credentials::check_source(&request.password_source)?;
            fields.runs_on = match &request.runs_on {
                Some(RunsOn::CloudSql { project, instance }) => {
                    let (project, instance) = (project.trim(), instance.trim());
                    if project.is_empty() || instance.is_empty() {
                        return Err(AppError::validation(
                            "Enter the Cloud SQL instance's project and name.",
                        ));
                    }
                    Some(RunsOn::CloudSql {
                        project: project.to_string(),
                        instance: instance.to_string(),
                    })
                }
                Some(RunsOn::Docker { container, socket }) => {
                    let container = container.trim();
                    if container.is_empty() {
                        return Err(AppError::validation("Choose the Docker container."));
                    }
                    Some(RunsOn::Docker {
                        container: container.to_string(),
                        socket: trimmed(socket),
                    })
                }
                None => None,
            };
        }
    }
    Ok(fields)
}

/// The fields of a `postgres://` or `postgresql://` URL. The password comes
/// back only to fill the form's password field.
pub fn parse_url(url: &str) -> AppResult<DbUrlFields> {
    let url = url.trim();
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| {
            AppError::validation("Paste a URL that starts with postgres:// or postgresql://.")
        })?;
    // The user and password first, up to the last `@`: a password pasted
    // without encoding can hold `/`, `?`, or `:`.
    let (userinfo, rest) = match rest.rsplit_once('@') {
        Some((u, r)) => (Some(u), r),
        None => (None, rest),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (rest, None),
    };
    let (hostport, database) = match rest.split_once('/') {
        Some((a, d)) => (a, Some(d)),
        None => (rest, None),
    };
    let (user, password) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((user, password)) => (Some(decode(user)?), Some(decode(password)?)),
            None => (Some(decode(u)?), None),
        },
        None => (None, None),
    };
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (host, after) = v6
            .split_once(']')
            .ok_or_else(|| AppError::validation("The URL's IPv6 address has no closing ]."))?;
        (host.to_string(), after.strip_prefix(':'))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (hostport.to_string(), None),
        }
    };
    let port = match port.filter(|p| !p.is_empty()) {
        Some(p) => Some(
            p.parse::<u16>()
                .map_err(|_| AppError::validation("The URL's port is not a number."))?,
        ),
        None => None,
    };
    let mut tls = None;
    for pair in query.unwrap_or("").split('&') {
        if let Some(mode) = pair.strip_prefix("sslmode=") {
            tls = match mode {
                "disable" => Some(DbTls::Off),
                "require" => Some(DbTls::Require),
                "verify-ca" | "verify-full" => Some(DbTls::Verify),
                _ => None,
            };
        }
    }
    let nonempty = |s: String| (!s.is_empty()).then_some(s);
    Ok(DbUrlFields {
        host: nonempty(decode(&host)?),
        port,
        database: database.map(decode).transpose()?.and_then(nonempty),
        user: user.and_then(nonempty),
        password: password.and_then(nonempty),
        tls,
    })
}

fn decode(s: &str) -> AppResult<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| {
                    AppError::validation("The URL has a % that is not followed by two hex digits.")
                })?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .map_err(|_| AppError::validation("The URL is not valid UTF-8 once decoded."))
}

// --- Rows -------------------------------------------------------------------

const COLUMNS: &str = "id, name, kind, environment, access, file_path, host, port, database, \
    user_name, tls, ca_file, secret_source, statement_timeout_seconds, version, runs_on, \
    credential_revision, source_approved, credential_pending";

fn parse<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn word<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<DbConnection> {
    Ok(DbConnection {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: parse(r.get(2)?)?,
        environment: parse(r.get(3)?)?,
        access: parse(r.get(4)?)?,
        file_path: r.get(5)?,
        host: r.get(6)?,
        port: r.get(7)?,
        database: r.get(8)?,
        user: r.get(9)?,
        tls: r.get::<_, Option<String>>(10)?.map(parse).transpose()?,
        ca_file: r.get(11)?,
        password: parse_json(r.get(12)?)?,
        password_ready: true,
        statement_timeout_seconds: r.get(13)?,
        file_size: None,
        repository_ids: Vec::new(),
        version: r.get(14)?,
        runs_on: r
            .get::<_, Option<String>>(15)?
            .and_then(|j| serde_json::from_str(&j).ok()),
        credential: CredentialState {
            revision: r.get(16)?,
            needs_approval: !r.get::<_, bool>(17)?,
            pending: r.get::<_, Option<String>>(18)?.map(parse).transpose()?,
        },
    })
}

fn repositories_of(conn: &Connection, id: &str) -> AppResult<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT repository_id FROM db_connection_repositories WHERE connection_id = ?1 ORDER BY repository_id",
    )?;
    let ids = statement.query_map([id], |r| r.get(0))?;
    Ok(ids.collect::<Result<_, _>>()?)
}

fn list(conn: &Connection) -> AppResult<Vec<DbConnection>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM db_connections ORDER BY name COLLATE NOCASE, created_at"
    ))?;
    let rows: Vec<DbConnection> = statement
        .query_map([], from_row)?
        .collect::<Result<_, _>>()?;
    rows.into_iter()
        .map(|mut c| {
            c.repository_ids = repositories_of(conn, &c.id)?;
            Ok(c)
        })
        .collect()
}

fn get(conn: &Connection, id: &str) -> AppResult<Option<DbConnection>> {
    let found = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM db_connections WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?;
    match found {
        Some(mut c) => {
            c.repository_ids = repositories_of(conn, &c.id)?;
            Ok(Some(c))
        }
        None => Ok(None),
    }
}

/// Link or unlink a connection and a repository.
fn set_link(conn: &Connection, id: &str, repository_id: &str, linked: bool) -> AppResult<()> {
    if linked {
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM repositories WHERE id = ?1)",
            [repository_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(AppError::not_found(
                "That repository is no longer in Brainiac.",
            ));
        }
        conn.execute(
            "INSERT OR IGNORE INTO db_connection_repositories (connection_id, repository_id) VALUES (?1, ?2)",
            params![id, repository_id],
        )?;
    } else {
        conn.execute(
            "DELETE FROM db_connection_repositories WHERE connection_id = ?1 AND repository_id = ?2",
            params![id, repository_id],
        )?;
    }
    Ok(())
}

fn source_json(source: &SecretSource) -> AppResult<String> {
    Ok(serde_json::to_string(source)?)
}

/// Mark a change under way, checking the version the user saw.
fn mark_pending(
    conn: &Connection,
    id: &str,
    pending: CredentialPending,
    expected_version: Option<i64>,
) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE db_connections SET credential_pending = ?2
          WHERE id = ?1 AND (?3 IS NULL OR version = ?3)",
        params![id, word(&pending), expected_version],
    )?;
    if changed == 0 {
        return Err(conflict());
    }
    Ok(())
}

/// A new connection whose password is being written: kept, but blocked,
/// until the save finishes.
fn insert_provisional(conn: &Connection, id: &str, f: &Fields, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO db_connections (id, name, kind, environment, access, file_path, host,
            port, database, user_name, tls, ca_file, secret_source, statement_timeout_seconds,
            version, created_at, updated_at, runs_on, credential_revision, credential_pending)
          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, ?15, ?15, ?16, 0, 'save')",
        params![
            id,
            f.name,
            word(&f.kind),
            word(&f.environment),
            word(&f.access),
            f.file_path,
            f.host,
            f.port,
            f.database,
            f.user,
            f.tls.as_ref().map(word),
            f.ca_file,
            source_json(&f.password)?,
            f.timeout,
            now,
            f.runs_on.as_ref().and_then(|r| serde_json::to_string(r).ok())
        ],
    )?;
    Ok(())
}

/// Write a connection's fields with its binding: the source, its revision,
/// approved by this save, and what is still pending. `bump` advances the
/// version of an existing connection; a new one starts at 1.
fn upsert(
    conn: &mut Connection,
    id: &str,
    f: &Fields,
    revision: i64,
    pending: Option<CredentialPending>,
    bump: bool,
    now: &str,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM db_connections WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    let source = source_json(&f.password)?;
    let pending = pending.as_ref().map(word);
    if exists {
        tx.execute(
            "UPDATE db_connections SET name = ?2, kind = ?3, environment = ?4, access = ?5,
                file_path = ?6, host = ?7, port = ?8, database = ?9, user_name = ?10, tls = ?11,
                ca_file = ?12, secret_source = ?13, statement_timeout_seconds = ?14,
                version = version + ?15, updated_at = ?16, runs_on = ?17,
                credential_revision = ?18, source_approved = 1, credential_pending = ?19
              WHERE id = ?1",
            params![
                id,
                f.name,
                word(&f.kind),
                word(&f.environment),
                word(&f.access),
                f.file_path,
                f.host,
                f.port,
                f.database,
                f.user,
                f.tls.as_ref().map(word),
                f.ca_file,
                source,
                f.timeout,
                i64::from(bump),
                now,
                f.runs_on
                    .as_ref()
                    .and_then(|r| serde_json::to_string(r).ok()),
                revision,
                pending,
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO db_connections (id, name, kind, environment, access, file_path, host,
                port, database, user_name, tls, ca_file, secret_source, statement_timeout_seconds,
                version, created_at, updated_at, runs_on, credential_revision, credential_pending)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, ?15, ?15, ?16, ?17, ?18)",
            params![
                id,
                f.name,
                word(&f.kind),
                word(&f.environment),
                word(&f.access),
                f.file_path,
                f.host,
                f.port,
                f.database,
                f.user,
                f.tls.as_ref().map(word),
                f.ca_file,
                source,
                f.timeout,
                now,
                f.runs_on.as_ref().and_then(|r| serde_json::to_string(r).ok()),
                revision,
                pending,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn clear_pending(conn: &Connection, id: &str, which: CredentialPending) -> AppResult<()> {
    conn.execute(
        "UPDATE db_connections SET credential_pending = NULL WHERE id = ?1 AND credential_pending = ?2",
        params![id, word(&which)],
    )?;
    Ok(())
}

fn approve(conn: &Connection, id: &str, revision: i64) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE db_connections SET source_approved = 1 WHERE id = ?1 AND credential_revision = ?2",
        params![id, revision],
    )?;
    if changed == 0 {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "This connection's source changed since it was shown. Look at it again.",
        ));
    }
    Ok(())
}

fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM db_connections WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_fill_the_form_and_hand_over_the_password() {
        let f =
            parse_url("postgres://app%40corp:p%40ss:w@db.example.com:6543/billing?sslmode=require")
                .unwrap();
        assert_eq!(f.host.as_deref(), Some("db.example.com"));
        assert_eq!(f.port, Some(6543));
        assert_eq!(f.database.as_deref(), Some("billing"));
        assert_eq!(f.user.as_deref(), Some("app@corp"));
        assert_eq!(f.password.as_deref(), Some("p@ss:w"));
        assert_eq!(f.tls, Some(DbTls::Require));
        assert!(!format!("{f:?}").contains("p@ss"));

        let v6 = parse_url("postgresql://[::1]/postgres").unwrap();
        assert_eq!((v6.host.as_deref(), v6.port), (Some("::1"), None));
        assert_eq!(v6.user, None);

        assert!(parse_url("mysql://x").is_err());
        assert!(parse_url("postgres://h:notaport/db").is_err());
        // A password with an unencoded `/` or `?` is read whole, never shown in an error.
        let odd = parse_url("postgres://app:s3cr3t/x?y@db.example.com/billing").unwrap();
        assert_eq!(odd.password.as_deref(), Some("s3cr3t/x?y"));
        assert_eq!(odd.host.as_deref(), Some("db.example.com"));
        assert_eq!(odd.database.as_deref(), Some("billing"));
        let err = parse_url("postgres://app:s3cr3t@db.example.com:x/billing").unwrap_err();
        assert!(!err.message.contains("s3cr3t"));
    }
}
