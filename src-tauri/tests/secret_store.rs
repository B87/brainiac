//! A run host's secret store (docs/design/scheduled-runs.md): sealed files in
//! the controller's folder, a name check on every call, and an honest refusal
//! on a host that cannot seal. `fake_creds` stands in for `systemd-creds`.

mod fake_engine;

use std::os::unix::fs::PermissionsExt;

use brainiac_lib::agents::controller::secret_store::SecretStore;
use brainiac_lib::models::ErrorCode;

const VALUE: &str = "sntrys_ExampleSentryTokenForTests0123456789";

#[tokio::test]
async fn a_secret_is_sealed_on_disk_listed_by_name_and_opened_for_a_run() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), fake_engine::fake_creds(tmp.path()));
    assert!(store.list().is_empty());

    store.put("SENTRY_TOKEN", VALUE).await.unwrap();
    let file = tmp.path().join("secrets/SENTRY_TOKEN.cred");
    let sealed = std::fs::read_to_string(&file).unwrap();
    assert_ne!(sealed, VALUE, "the value is not on disk as it was given");
    // Closed to other users, the file and its folder.
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(tmp.path().join("secrets"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let listed = store.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "SENTRY_TOKEN");
    assert!(listed[0].set_at > 1_700_000_000);
    assert_eq!(store.get("SENTRY_TOKEN").await.unwrap(), VALUE);

    // Setting it again replaces it.
    store
        .put("SENTRY_TOKEN", "a-newer-token-value")
        .await
        .unwrap();
    assert_eq!(
        store.get("SENTRY_TOKEN").await.unwrap(),
        "a-newer-token-value"
    );
    assert_eq!(store.list().len(), 1);

    store.delete("SENTRY_TOKEN").unwrap();
    assert!(store.list().is_empty());
    assert_eq!(
        store.get("SENTRY_TOKEN").await.unwrap_err().code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn names_and_values_are_checked_and_nothing_is_stored_when_they_are_not() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), fake_engine::fake_creds(tmp.path()));
    for name in [
        "sentry",
        "PATH",
        "../ESCAPE",
        "A/B",
        "",
        "NODE_OPTIONS",
        "HTTPS_PROXY",
    ] {
        assert!(store.put(name, VALUE).await.is_err(), "{name:?}");
        assert!(store.get(name).await.is_err(), "{name:?}");
        assert!(store.delete(name).is_err(), "{name:?}");
    }
    assert!(store.put("SENTRY_TOKEN", "short").await.is_err());
    assert!(store.put("SENTRY_TOKEN", "has\0nul-in-it").await.is_err());
    assert!(!tmp.path().join("secrets").exists() || store.list().is_empty());
}

#[tokio::test]
async fn a_host_that_cannot_seal_keeps_nothing_and_says_so_without_the_value() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), tmp.path().join("no-such-program"));
    let error = store.put("SENTRY_TOKEN", VALUE).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::DependencyUnavailable);
    assert!(error.message.contains("cannot keep secrets"), "{error:?}");
    assert!(!format!("{error:?}").contains(VALUE));
    assert!(store.list().is_empty(), "no plain file was written instead");
}
