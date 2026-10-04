//! Saved connections (SPEC.md, Databases: Connections): rows in the core
//! database, passwords in the Keychain or asked for once per run, and Test
//! Connection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::driver::{Session, Target};
use super::postgres::PgTarget;
use crate::credentials::{Secret, Secrets};
use crate::db::Db;
use crate::models::{
    now_rfc3339, AppError, AppResult, DbAccess, DbConnection, DbEnvironment, DbKind, DbPassword,
    DbTestResult, DbTls, DbUrlFields, ErrorCode, RunsOn, SaveDbConnectionRequest,
};

const DEFAULT_PORT: u16 = 5432;
const MAX_TIMEOUT_SECONDS: u32 = 3600;

/// The Keychain account of a connection's password.
pub fn keychain_account(id: &str) -> String {
    format!("db:{id}")
}

pub struct ConnectionService {
    db: Db,
    // `Arc<dyn Secrets>`: the real Keychain in the app, memory in tests,
    // shared with the blocking threads that call it.
    secrets: Arc<dyn Secrets>,
    /// Passwords read from the Keychain or entered for `Ask`, by connection,
    /// for this run only: each Keychain read may ask the user to allow it.
    passwords: Mutex<HashMap<String, Secret>>,
}

impl ConnectionService {
    pub fn new(db: Db, secrets: Arc<dyn Secrets>) -> Self {
        ConnectionService {
            db,
            secrets,
            passwords: Mutex::new(HashMap::new()),
        }
    }

    /// Run a Keychain call on a blocking thread: it may wait for the user to
    /// answer macOS's prompt.
    async fn secrets<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn Secrets) -> AppResult<T> + Send + 'static,
    {
        let secrets = Arc::clone(&self.secrets);
        tokio::task::spawn_blocking(move || f(secrets.as_ref()))
            .await
            .map_err(|e| {
                AppError::io("The Keychain call did not finish.").with_details(e.to_string())
            })?
    }

    fn remembered(&self, id: &str) -> Option<Secret> {
        self.passwords
            .lock()
            .expect("passwords lock")
            .get(id)
            .cloned()
    }

    fn remember(&self, id: &str, secret: Secret) {
        self.passwords
            .lock()
            .expect("passwords lock")
            .insert(id.to_string(), secret);
    }

    /// Forget a password kept for this run, so it is read or asked for again.
    pub fn forget(&self, id: &str) {
        self.passwords.lock().expect("passwords lock").remove(id);
    }

    fn complete(&self, mut connection: DbConnection) -> DbConnection {
        connection.password_ready = match connection.password {
            DbPassword::Ask => self.remembered(&connection.id).is_some(),
            _ => true,
        };
        connection.file_size = connection
            .file_path
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        connection
    }

    pub async fn list(&self) -> AppResult<Vec<DbConnection>> {
        let rows = self.db.call(|conn| list(conn)).await?;
        Ok(rows.into_iter().map(|c| self.complete(c)).collect())
    }

    pub async fn get(&self, id: &str) -> AppResult<DbConnection> {
        let key = id.to_string();
        let row = self.db.call(move |conn| get(conn, &key)).await?;
        row.map(|c| self.complete(c))
            .ok_or_else(|| AppError::not_found("That connection no longer exists."))
    }

    /// Create or update a connection. A typed password goes to the Keychain
    /// (or, for `Ask`, is kept for this run); it is never stored here.
    pub async fn save(&self, request: SaveDbConnectionRequest) -> AppResult<DbConnection> {
        let stored = match &request.id {
            Some(id) => Some(self.get(id).await?),
            None => None,
        };
        let fields = validate(&request)?;
        let password = request.password.as_deref().map(Secret::new).transpose()?;
        if fields.password == DbPassword::Keychain && password.is_none() {
            let kept = stored
                .as_ref()
                .is_some_and(|s| s.password == DbPassword::Keychain);
            if !kept {
                return Err(AppError::validation(
                    "Enter the password to keep in the Keychain.",
                ));
            }
        }
        let id = stored
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let account = keychain_account(&id);
        match fields.password {
            DbPassword::Keychain => {
                if let Some(secret) = password.clone() {
                    let account = account.clone();
                    self.secrets(move |s| s.set(&account, &secret)).await?;
                }
            }
            DbPassword::Ask | DbPassword::None => {
                if stored
                    .as_ref()
                    .is_some_and(|s| s.password == DbPassword::Keychain)
                {
                    let account = account.clone();
                    self.secrets(move |s| s.delete(&account)).await?;
                }
            }
        }
        match (&password, fields.password) {
            (Some(secret), DbPassword::Keychain | DbPassword::Ask) => {
                self.remember(&id, secret.clone())
            }
            (None, DbPassword::Keychain) => {}
            _ => self.forget(&id),
        }
        let now = now_rfc3339();
        let row_id = id.clone();
        let expected = request.expected_version;
        let saved = self
            .db
            .call(move |conn| {
                upsert(conn, &row_id, &fields, expected, &now)?;
                get(conn, &row_id)
            })
            .await;
        match saved {
            Ok(Some(connection)) => {
                tracing::info!(connection = %connection.id, kind = connection.kind.as_str(), "database connection saved");
                Ok(self.complete(connection))
            }
            Ok(None) => Err(AppError::db("The connection was not saved.")),
            Err(e) => {
                if stored.is_none() && password.is_some() {
                    // Do not leave a password behind for a connection that does not exist.
                    let _ = self.secrets(move |s| s.delete(&account)).await;
                }
                Err(e)
            }
        }
    }

    /// Delete a connection and its Keychain item.
    pub async fn delete(&self, id: &str, expected_version: i64) -> AppResult<()> {
        let stored = self.get(id).await?;
        if stored.version != expected_version {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This connection was changed since it was shown. Look at it again before deleting it.",
            ));
        }
        let key = id.to_string();
        self.db.call(move |conn| delete(conn, &key)).await?;
        if stored.password == DbPassword::Keychain {
            let account = keychain_account(id);
            self.secrets(move |s| s.delete(&account)).await?;
        }
        self.forget(id);
        tracing::info!(connection = %id, "database connection deleted");
        Ok(())
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

    /// Keep the password of an `Ask` connection for this run.
    pub async fn unlock(&self, id: &str, password: &str) -> AppResult<DbConnection> {
        let connection = self.get(id).await?;
        self.remember(id, Secret::new(password)?);
        Ok(self.complete(connection))
    }

    /// Test Connection: connect once with the form's fields and report the
    /// server's version.
    pub async fn test(&self, request: SaveDbConnectionRequest) -> AppResult<DbTestResult> {
        let fields = validate(&request)?;
        let password = match (request.password.as_deref(), fields.password) {
            (_, DbPassword::None) => None,
            (Some(typed), _) => Some(Secret::new(typed)?),
            (None, _) => match &request.id {
                Some(id) => self.password(id, fields.password).await.ok(),
                None => None,
            },
        };
        let target = target(&fields, password);
        let session = Session::open(&target).await?;
        Ok(DbTestResult {
            server_version: session.server_version().to_string(),
        })
    }

    /// The password to connect with: kept for this run, else from the Keychain.
    async fn password(&self, id: &str, storage: DbPassword) -> AppResult<Secret> {
        if let Some(secret) = self.remembered(id) {
            return Ok(secret);
        }
        match storage {
            DbPassword::Keychain => {
                let account = keychain_account(id);
                let secret = self.secrets(move |s| s.get(&account)).await?.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::PermissionDenied,
                        "The connection's password is no longer in the Keychain. Edit the connection and enter it again.",
                    )
                })?;
                self.remember(id, secret.clone());
                Ok(secret)
            }
            _ => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Enter the connection's password to connect.",
            )),
        }
    }

    /// What a session for this connection connects to.
    pub async fn target(&self, connection: &DbConnection) -> AppResult<Target> {
        let fields = Fields::from(connection);
        let password = match connection.password {
            DbPassword::None => None,
            storage => Some(self.password(&connection.id, storage).await?),
        };
        Ok(target(&fields, password))
    }
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
    password: DbPassword,
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
            password: c.password,
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
        password: DbPassword::None,
        timeout: request.statement_timeout_seconds,
        runs_on: None,
    };
    match request.kind {
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
            fields.password = request.password_storage;
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
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (rest, None),
    };
    let (authority, database) = match rest.split_once('/') {
        Some((a, d)) => (a, Some(d)),
        None => (rest, None),
    };
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
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
                .map_err(|_| AppError::validation(format!("{p} is not a port number.")))?,
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
    user_name, tls, ca_file, password, statement_timeout_seconds, version, runs_on";

fn parse<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|e| {
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
        password: parse(r.get(12)?)?,
        password_ready: true,
        statement_timeout_seconds: r.get(13)?,
        file_size: None,
        repository_ids: Vec::new(),
        version: r.get(14)?,
        runs_on: r
            .get::<_, Option<String>>(15)?
            .and_then(|j| serde_json::from_str(&j).ok()),
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

fn upsert(
    conn: &mut Connection,
    id: &str,
    f: &Fields,
    expected_version: Option<i64>,
    now: &str,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    let exists: Option<i64> = tx
        .query_row(
            "SELECT version FROM db_connections WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    match exists {
        Some(version) => {
            if expected_version.is_some_and(|e| e != version) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This connection was changed since the form opened. Close the form and open it again.",
                ));
            }
            tx.execute(
                "UPDATE db_connections SET name = ?2, kind = ?3, environment = ?4, access = ?5,
                    file_path = ?6, host = ?7, port = ?8, database = ?9, user_name = ?10, tls = ?11,
                    ca_file = ?12, password = ?13, statement_timeout_seconds = ?14,
                    version = version + 1, updated_at = ?15, runs_on = ?16
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
                    word(&f.password),
                    f.timeout,
                    now,
                    f.runs_on
                        .as_ref()
                        .and_then(|r| serde_json::to_string(r).ok())
                ],
            )?;
        }
        None => {
            tx.execute(
                "INSERT INTO db_connections (id, name, kind, environment, access, file_path, host,
                    port, database, user_name, tls, ca_file, password, statement_timeout_seconds,
                    version, created_at, updated_at, runs_on)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1, ?15, ?15, ?16)",
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
                    word(&f.password),
                    f.timeout,
                    now,
                    f.runs_on.as_ref().and_then(|r| serde_json::to_string(r).ok())
                ],
            )?;
        }
    }
    tx.commit()?;
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
    }
}
