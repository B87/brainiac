//! Machines (docs/architecture.md, Machines): a remote host's connection and
//! approved key belong to its machine; the run host is a role on it.

use brainiac_lib::agents::hosts;
use brainiac_lib::db::{self, Db};

const NOW: &str = "2026-10-06T09:00:00Z";

#[tokio::test(flavor = "multi_thread")]
async fn a_new_database_has_this_mac_and_no_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
    let listed = core.call(|conn| hosts::list(conn)).await.unwrap();
    assert_eq!(listed.len(), 1);
    let local = &listed[0];
    assert_eq!(
        (local.id.as_str(), local.name.as_str(), local.approved),
        (hosts::LOCAL_ID, "This Mac", true)
    );
    assert!(local.ssh_host.is_none() && local.fingerprint.is_none());
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
