//! Machines (docs/architecture.md, Machines): a remote host's connection and
//! approved key belong to its machine; the run host is a role on it.

use brainiac_lib::agents::hosts;
use brainiac_lib::db::{self, Db};
use rusqlite::Connection;

const NOW: &str = "2026-10-06T09:00:00Z";

/// A development database from before machines: twelve migrations, and two
/// SSH hosts, one installed and one removed with its work kept.
fn before_machines(path: &std::path::Path) {
    let mut conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "application_id", db::APPLICATION_ID)
        .unwrap();
    let twelve = db::Store {
        migrations: &db::CORE.migrations[..12],
        ..db::CORE
    };
    db::migrate(&mut conn, &twelve).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO agent_hosts (id, kind, name, ssh_user, ssh_host, ssh_port, identity_path,
            fingerprint, installation, protocol, approved, engine_name, loop_devices,
            controller_build, created_at, updated_at)
         VALUES ('h1', 'ssh', 'build-01', 'ada', 'runner.example', 2222, '/Users/ada/.ssh/id_ed25519',
            'SHA256:abc', 'inst-1', 1, 1, 'Docker Engine 27', 1, 'b1', '{NOW}', '{NOW}');
         INSERT INTO agent_hosts (id, kind, name, ssh_user, ssh_host, ssh_port, fingerprint,
            approved, state_kept, created_at, updated_at)
         VALUES ('h2', 'ssh', 'build-02', 'ada', 'build-02.example', 22, 'SHA256:def', 0, 1,
            '{NOW}', '{NOW}');"
    ))
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_development_database_moves_each_ssh_host_onto_a_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(db::CORE_FILE);
    before_machines(&path);
    let core = Db::open(&path).unwrap();

    let listed = core.call(|conn| hosts::list(conn)).await.unwrap();
    let local = listed.iter().find(|h| h.id == hosts::LOCAL_ID).unwrap();
    assert_eq!((local.name.as_str(), local.approved), ("This Mac", true));
    assert!(local.ssh_host.is_none() && local.fingerprint.is_none());

    let installed = listed.iter().find(|h| h.id == "h1").unwrap();
    assert_eq!(installed.name, "build-01");
    assert_eq!(
        (
            installed.ssh_user.as_deref(),
            installed.ssh_host.as_deref(),
            installed.ssh_port,
            installed.identity_path.as_deref(),
            installed.fingerprint.as_deref(),
        ),
        (
            Some("ada"),
            Some("runner.example"),
            Some(2222),
            Some("/Users/ada/.ssh/id_ed25519"),
            Some("SHA256:abc"),
        )
    );
    // What was installed stays; the key waits to be confirmed again,
    // because its file moved to the machine's folder.
    assert!(installed.installed && installed.loop_devices);
    assert_eq!(installed.engine_name.as_deref(), Some("Docker Engine 27"));
    assert!(!installed.approved);

    let kept = listed.iter().find(|h| h.id == "h2").unwrap();
    assert!(kept.state_kept && !kept.approved);

    let (machines, broken, roles): (i64, i64, Vec<(String, Option<String>)>) = core
        .call(|conn| {
            let machines = conn.query_row("SELECT COUNT(*) FROM machines", [], |r| r.get(0))?;
            let broken =
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?;
            let mut stmt = conn.prepare("SELECT id, machine_id FROM agent_hosts ORDER BY id")?;
            let roles = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((machines, broken, roles))
        })
        .await
        .unwrap();
    assert_eq!((machines, broken), (2, 0));
    assert_eq!(
        roles,
        [
            ("h1".to_string(), Some("h1".to_string())),
            ("h2".to_string(), Some("h2".to_string())),
            ("local".to_string(), None),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_host_needs_a_machine_and_this_mac_has_none() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
    let refused = core
        .call(|conn| {
            let ssh_alone = conn
                .execute(
                    "INSERT INTO agent_hosts (id, kind, created_at, updated_at)
                     VALUES ('h1', 'ssh', ?1, ?1)",
                    [NOW],
                )
                .is_err();
            let missing_machine = conn
                .execute(
                    "INSERT INTO agent_hosts (id, kind, machine_id, created_at, updated_at)
                     VALUES ('h2', 'ssh', 'nowhere', ?1, ?1)",
                    [NOW],
                )
                .is_err();
            Ok((ssh_alone, missing_machine))
        })
        .await
        .unwrap();
    assert_eq!(refused, (true, true));
}
