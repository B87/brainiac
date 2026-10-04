//! Where connection passwords come from (SPEC.md, Secrets;
//! docs/design/secrets.md, exit gates of phase 1): the upgrade reads
//! nothing, an interrupted Keychain save stays blocked across a restart, a
//! move to a command never falls back to the old item and its cleanup
//! cannot delete a newer one, a removal that cannot delete the item is
//! retried, a restored source is read only once allowed, and every source is
//! read once per run until refreshed or refused. The Keychain is in memory.

mod postgres_server;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::databases::driver::Target;
use brainiac_lib::databases::ConnectionService;
use brainiac_lib::db::{self, Db};
use brainiac_lib::models::{
    CredentialOwner, CredentialPending, DbAccess, DbConnection, DbEnvironment, DbKind, DbTls,
    ErrorCode, SaveDbConnectionRequest, SecretSource,
};
use brainiac_lib::secrets::SecretsService;
use postgres_server::PgServer;

struct Harness {
    tmp: tempfile::TempDir,
    core: Db,
    store: Arc<MemoryStore>,
    connections: Arc<ConnectionService>,
}

impl Harness {
    fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
        let store = Arc::new(MemoryStore::default());
        let connections = Self::connections(&tmp, &core, &store);
        Harness {
            tmp,
            core,
            store,
            connections,
        }
    }

    fn connections(
        tmp: &tempfile::TempDir,
        core: &Db,
        store: &Arc<MemoryStore>,
    ) -> Arc<ConnectionService> {
        let credentials = Arc::new(CredentialService::new(
            store.clone(),
            CommandRunner::new(tmp.path().join("commands")),
        ));
        Arc::new(ConnectionService::new(core.clone(), credentials))
    }

    /// Brainiac quits and starts again: the same files, nothing in memory.
    fn restart(&mut self) {
        self.connections = Self::connections(&self.tmp, &self.core, &self.store);
    }

    /// A program that prints `password` and counts its runs in `<name>.runs`.
    fn script(&self, name: &str, password: &str) -> String {
        let path = self.tmp.path().join(name);
        let runs = self.tmp.path().join(format!("{name}.runs"));
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho run >> '{}'\nprintf '%s\\n' '{password}'\n",
                runs.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    fn runs(&self, name: &str) -> usize {
        std::fs::read_to_string(self.tmp.path().join(format!("{name}.runs")))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    async fn password(&self, connection: &DbConnection) -> Result<Option<String>, ErrorCode> {
        let connection = self.connections.get(&connection.id).await.unwrap();
        match self.connections.target(&connection).await {
            Ok((Target::Postgres(t), _)) => Ok(t.password.map(|p| p.expose().to_string())),
            Ok(_) => panic!("not PostgreSQL"),
            Err(e) => Err(e.code),
        }
    }

    async fn sql(&self, sql: &'static str) {
        self.core
            .call(move |conn| Ok(conn.execute_batch(sql)?))
            .await
            .unwrap();
    }
}

fn command(program: &str) -> SecretSource {
    SecretSource::Command {
        program: program.to_string(),
        args: vec![],
    }
}

/// A PostgreSQL connection; nothing listens on its port unless a server is given.
fn request(source: SecretSource, typed: Option<&str>, port: u16) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: None,
        expected_version: None,
        name: "Billing".into(),
        kind: DbKind::Postgres,
        environment: DbEnvironment::Local,
        access: DbAccess::ReadOnly,
        file_path: None,
        host: Some("127.0.0.1".into()),
        port: Some(port),
        database: Some("postgres".into()),
        user: Some("postgres".into()),
        tls: Some(DbTls::Off),
        ca_file: None,
        password_source: source,
        password: typed.map(str::to_string),
        statement_timeout_seconds: 30,
        runs_on: None,
    }
}

fn edit(
    saved: &DbConnection,
    source: SecretSource,
    typed: Option<&str>,
) -> SaveDbConnectionRequest {
    SaveDbConnectionRequest {
        id: Some(saved.id.clone()),
        expected_version: Some(saved.version),
        ..request(source, typed, saved.port.unwrap())
    }
}

/// A port nothing listens on.
fn closed_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_keeps_every_source_and_reads_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(db::CORE_FILE);
    {
        // A database as 0.4.0 leaves it: six migrations, and connections
        // whose passwords are in the Keychain, asked for, or none.
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "application_id", db::APPLICATION_ID)
            .unwrap();
        let six = db::Store {
            migrations: &db::CORE.migrations[..6],
            ..db::CORE
        };
        db::migrate(&mut conn, &six).unwrap();
        for (id, password) in [("a", "keychain"), ("b", "ask"), ("c", "none")] {
            conn.execute(
                "INSERT INTO db_connections (id, name, kind, environment, host, port, database,
                    user_name, tls, password, created_at, updated_at)
                 VALUES (?1, ?1, 'postgres', 'local', '127.0.0.1', 5432, 'postgres', 'postgres',
                    'off', ?2, '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z')",
                rusqlite::params![id, password],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO forge_accounts (id, kind, host, login, user_id, token_kind, checked_at, created_at)
             VALUES ('g', 'github', 'github.com', 'octo', '42', 'classic', '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z')",
            [],
        )
        .unwrap();
    }
    let core = Db::open(&path).unwrap();
    let store = Arc::new(MemoryStore::default());
    store.insert("db:a", "kept");
    let connections = Harness::connections(&tmp, &core, &store);
    let listed = connections.list().await.unwrap();
    let sources: Vec<SecretSource> = listed.iter().map(|c| c.password.clone()).collect();
    assert_eq!(
        sources,
        [SecretSource::Store, SecretSource::Ask, SecretSource::None]
    );
    assert!(listed
        .iter()
        .all(|c| !c.credential.needs_approval && c.credential.pending.is_none()));
    let accounts: Vec<(String, bool)> = core
        .call(|conn| {
            let mut s =
                conn.prepare("SELECT secret_source, source_approved FROM forge_accounts")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .await
        .unwrap();
    assert_eq!(accounts, [(r#"{"kind":"store"}"#.to_string(), true)]);
    assert_eq!(store.reads(), 0, "the upgrade read no secret");

    // The item is used as before, read once.
    let (target, _) = connections.target(&listed[0]).await.unwrap();
    let Target::Postgres(target) = target else {
        panic!()
    };
    assert_eq!(target.password.unwrap().expose(), "kept");
    connections.target(&listed[0]).await.unwrap();
    assert_eq!(store.reads(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_keychain_save_stays_blocked_until_saved_again() {
    let mut h = Harness::new();
    let port = closed_port();
    let saved = h
        .connections
        .save(request(SecretSource::Store, Some("first"), port))
        .await
        .unwrap();
    assert_eq!(h.password(&saved).await, Ok(Some("first".into())));

    h.store.fail_writes(true);
    let err = h
        .connections
        .save(edit(&saved, SecretSource::Store, Some("second")))
        .await
        .unwrap_err();
    assert!(
        err.message
            .contains("cannot be used until it is saved again"),
        "{err:?}"
    );
    let blocked = h.connections.get(&saved.id).await.unwrap();
    assert_eq!(blocked.credential.pending, Some(CredentialPending::Save));
    assert_eq!(h.password(&saved).await, Err(ErrorCode::PermissionDenied));

    // A restart does not resolve it, and keeping the item is not enough.
    h.restart();
    assert_eq!(h.password(&saved).await, Err(ErrorCode::PermissionDenied));
    let err = h
        .connections
        .save(edit(&blocked, SecretSource::Store, None))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);

    h.store.fail_writes(false);
    let fixed = h
        .connections
        .save(edit(&blocked, SecretSource::Store, Some("third")))
        .await
        .unwrap();
    assert_eq!(fixed.credential.pending, None);
    assert!(fixed.credential.revision > saved.credential.revision);
    assert_eq!(h.password(&fixed).await, Ok(Some("third".into())));

    // A new connection whose item cannot be written is not added, so
    // trying Save again does not leave a second one behind.
    h.store.fail_writes(true);
    for _ in 0..2 {
        let err = h
            .connections
            .save(request(SecretSource::Store, Some("new"), port))
            .await
            .unwrap_err();
        assert!(err.message.contains("was not added"), "{err:?}");
    }
    assert_eq!(h.connections.list().await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_cleanup_marker_never_deletes_this_macs_item_on_save() {
    let mut h = Harness::new();
    let program = h.script("op", "pw");
    let saved = h
        .connections
        .save(request(command(&program), None, closed_port()))
        .await
        .unwrap();
    let owner = format!("db:{}", saved.id);
    // Restored with a cleanup that had not finished, and an item of that
    // name in this Mac's Keychain.
    h.store.insert(&owner, "this mac's");
    h.sql("UPDATE db_connections SET source_approved = 0, credential_pending = 'cleanup'")
        .await;
    h.restart();
    let restored = h.connections.get(&saved.id).await.unwrap();
    let renamed = h
        .connections
        .save(SaveDbConnectionRequest {
            name: "Renamed".into(),
            ..edit(&restored, command(&program), None)
        })
        .await
        .unwrap();
    assert_eq!(renamed.credential.pending, Some(CredentialPending::Cleanup));
    assert_eq!(h.store.text(&owner).as_deref(), Some("this mac's"));
    // Only an explicit Retry deletes it.
    h.connections.retry_cleanup(&saved.id).await.unwrap();
    assert_eq!(h.store.text(&owner), None);

    // A restored Keychain connection moved to a command keeps the item too.
    let kept = h
        .connections
        .save(request(SecretSource::Store, Some("pw"), closed_port()))
        .await
        .unwrap();
    let kept_owner = format!("db:{}", kept.id);
    h.sql("UPDATE db_connections SET source_approved = 0").await;
    h.restart();
    let restored = h.connections.get(&kept.id).await.unwrap();
    let moved = h
        .connections
        .save(edit(&restored, command(&program), None))
        .await
        .unwrap();
    assert_eq!(moved.credential.pending, Some(CredentialPending::Cleanup));
    assert_eq!(h.store.text(&kept_owner).as_deref(), Some("pw"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_while_a_connection_is_in_use_is_picked_up_not_refused() {
    let h = Harness::new();
    let first = h.script("op", "first");
    let second = h.script("bw", "second");
    let saved = h
        .connections
        .save(request(command(&first), None, closed_port()))
        .await
        .unwrap();
    // A caller read the connection, then it was saved with another source.
    let before = h.connections.get(&saved.id).await.unwrap();
    h.password(&before).await.unwrap();
    h.connections
        .save(edit(&before, command(&second), None))
        .await
        .unwrap();
    let (target, _) = h.connections.target(&before).await.unwrap();
    let Target::Postgres(target) = target else {
        panic!()
    };
    assert_eq!(target.password.unwrap().expose(), "second");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_to_a_command_and_back_never_uses_or_deletes_the_wrong_item() {
    let h = Harness::new();
    let port = closed_port();
    let program = h.script("op", "from-command");
    let saved = h
        .connections
        .save(request(SecretSource::Store, Some("kept"), port))
        .await
        .unwrap();
    let owner = format!("db:{}", saved.id);

    h.store.fail_deletes(true);
    let moved = h
        .connections
        .save(edit(&saved, command(&program), None))
        .await
        .unwrap();
    assert_eq!(moved.credential.pending, Some(CredentialPending::Cleanup));
    assert_eq!(h.store.text(&owner).as_deref(), Some("kept"));
    assert_eq!(h.password(&moved).await, Ok(Some("from-command".into())));

    // Back to the Keychain before the cleanup ran: the save clears it, and
    // a retry cannot delete the new item.
    let back = h
        .connections
        .save(edit(&moved, SecretSource::Store, Some("new")))
        .await
        .unwrap();
    assert_eq!(back.credential.pending, None);
    h.connections.retry_cleanup(&saved.id).await.unwrap();
    assert_eq!(h.store.text(&owner).as_deref(), Some("new"));
    assert_eq!(h.password(&back).await, Ok(Some("new".into())));

    // Away again; once deleting works, Retry finishes the cleanup.
    let away = h
        .connections
        .save(edit(&back, command(&program), None))
        .await
        .unwrap();
    assert_eq!(away.credential.pending, Some(CredentialPending::Cleanup));
    h.store.fail_deletes(false);
    h.connections.retry_cleanup(&saved.id).await.unwrap();
    assert_eq!(h.store.text(&owner), None);
    let done = h.connections.get(&saved.id).await.unwrap();
    assert_eq!(done.credential.pending, None);
    assert_eq!(h.password(&done).await, Ok(Some("from-command".into())));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removal_that_cannot_delete_the_item_stays_visible_until_retried() {
    let h = Harness::new();
    let saved = h
        .connections
        .save(request(SecretSource::Store, Some("pw"), closed_port()))
        .await
        .unwrap();
    h.store.fail_deletes(true);
    let err = h
        .connections
        .delete(&saved.id, saved.version)
        .await
        .unwrap_err();
    assert!(
        err.message.contains("Retry in Settings → Secrets"),
        "{err:?}"
    );
    assert!(h.connections.list().await.unwrap().is_empty());
    assert!(h.connections.is_removed(&saved.id).await);
    assert_eq!(h.password(&saved).await, Err(ErrorCode::PermissionDenied));
    let entries = h.connections.secret_entries().await.unwrap();
    assert_eq!(entries[0].state.pending, Some(CredentialPending::Removal));

    h.store.fail_deletes(false);
    h.connections.retry_cleanup(&saved.id).await.unwrap();
    assert!(h.connections.get(&saved.id).await.is_err());
    assert_eq!(h.store.text(&format!("db:{}", saved.id)), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_source_is_read_only_once_allowed() {
    let mut h = Harness::new();
    let program = h.script("op", "pw");
    let saved = h
        .connections
        .save(request(command(&program), None, closed_port()))
        .await
        .unwrap();
    // What a restore does to every connection.
    h.sql("UPDATE db_connections SET source_approved = 0").await;
    h.restart();
    let restored = h.connections.get(&saved.id).await.unwrap();
    assert!(restored.credential.needs_approval);
    assert_eq!(
        h.password(&restored).await,
        Err(ErrorCode::PermissionDenied)
    );
    let tested = h
        .connections
        .test(edit(&restored, command(&program), None))
        .await
        .unwrap_err();
    assert_eq!(tested.code, ErrorCode::PermissionDenied);
    assert_eq!(h.runs("op"), 0, "nothing ran before the source was allowed");

    let err = h
        .connections
        .approve(&saved.id, restored.credential.revision + 1)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    let allowed = h
        .connections
        .approve(&saved.id, restored.credential.revision)
        .await
        .unwrap();
    assert!(!allowed.credential.needs_approval);
    assert_eq!(h.password(&allowed).await, Ok(Some("pw".into())));
    assert_eq!(h.runs("op"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn sources_are_read_once_per_run_until_refreshed_and_test_reads_afresh() {
    let h = Harness::new();
    let port = closed_port();
    let name = format!("BRAINIAC_TEST_DB_PASSWORD_{}", std::process::id());
    std::env::set_var(&name, "from-env");
    let env = h
        .connections
        .save(request(
            SecretSource::Environment { name: name.clone() },
            None,
            port,
        ))
        .await
        .unwrap();
    assert_eq!(h.password(&env).await, Ok(Some("from-env".into())));
    std::env::set_var(&name, "rotated");
    assert_eq!(h.password(&env).await, Ok(Some("from-env".into())));
    h.connections.refresh(&env.id);
    assert_eq!(h.password(&env).await, Ok(Some("rotated".into())));
    std::env::remove_var(&name);

    let program = h.script("op", "from-command");
    let cmd = h
        .connections
        .save(request(command(&program), None, port))
        .await
        .unwrap();
    h.password(&cmd).await.unwrap();
    h.password(&cmd).await.unwrap();
    assert_eq!(h.runs("op"), 1);

    // Test reads the draft afresh and leaves the run's value alone; its
    // outcome is shown for the saved binding.
    let tested = h
        .connections
        .test(edit(&cmd, command(&program), None))
        .await;
    assert!(tested.is_err(), "nothing listens on the port");
    assert_eq!(h.runs("op"), 2);
    h.password(&cmd).await.unwrap();
    assert_eq!(h.runs("op"), 2);
    let entries = h.connections.secret_entries().await.unwrap();
    let entry = entries
        .iter()
        .find(|e| e.owner == CredentialOwner::DbConnection { id: cmd.id.clone() })
        .unwrap();
    assert!(!entry.last_test.as_ref().unwrap().ok);

    // A different draft is not the saved binding's test.
    let other = h.script("bw", "other");
    let _ = h.connections.test(edit(&cmd, command(&other), None)).await;
    assert_eq!(h.runs("bw"), 1);
    let entries = h.connections.secret_entries().await.unwrap();
    let entry = entries
        .iter()
        .find(|e| {
            e.label == "Billing"
                && matches!(&e.source, SecretSource::Command { program: p, .. } if *p == program)
        })
        .unwrap();
    assert!(entry.last_test.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_secrets_reads_no_secret_and_runs_nothing() {
    let mut h = Harness::new();
    let program = h.script("op", "pw");
    h.connections
        .save(request(SecretSource::Store, Some("pw"), closed_port()))
        .await
        .unwrap();
    h.connections
        .save(request(command(&program), None, closed_port()))
        .await
        .unwrap();
    h.restart();
    let reads = h.store.reads();
    let credentials = Arc::new(CredentialService::new(
        h.store.clone(),
        CommandRunner::new(h.tmp.path().join("commands")),
    ));
    let accounts = Arc::new(brainiac_lib::forge::AccountService::new(
        h.core.clone(),
        Arc::clone(&credentials),
        brainiac_lib::forge::http::Http::insecure_for_tests().unwrap(),
        brainiac_lib::forge::Endpoints::production(),
    ));
    let secrets = SecretsService::new(credentials, accounts, Arc::clone(&h.connections));
    let overview = secrets.overview().await.unwrap();
    assert_eq!(overview.store, "Memory (tests)");
    assert_eq!(overview.entries.len(), 2);
    assert!(overview.entries.iter().all(|e| e.last_test.is_none()));
    assert_eq!(h.store.reads(), reads);
    assert_eq!(h.runs("op"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_password_the_server_refuses_is_read_again() {
    let Some(server) = PgServer::start() else {
        return;
    };
    let h = Harness::new();
    let file = h.tmp.path().join("password.txt");
    std::fs::write(&file, "wrong").unwrap();
    let saved = h
        .connections
        .save(request(
            SecretSource::Command {
                program: "/bin/cat".into(),
                args: vec![file.display().to_string()],
            },
            None,
            server.port,
        ))
        .await
        .unwrap();
    let err = h.connections.open(&saved).await.err().unwrap();
    assert_eq!(err.code, ErrorCode::Unauthenticated);
    std::fs::write(&file, postgres_server::PASSWORD).unwrap();
    // The refused lease was forgotten, so the command runs again.
    let session = h.connections.open(&saved).await.unwrap();
    assert!(session.server_version().starts_with("PostgreSQL"));
}
