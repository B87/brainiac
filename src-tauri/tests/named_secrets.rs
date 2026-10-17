//! Named secrets and Share (docs/design/scheduled-runs.md): a secret kept in
//! Settings → Secrets is sent to a run host, which seals a copy; the host's
//! page tells shared from out of date, missing, and failed; Remove takes it
//! back. The host is a real controller with a stand-in for `systemd-creds`,
//! the Keychain is in memory.

mod fake_engine;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{serve, Config};
use brainiac_lib::agents::named_secrets::NamedSecretService;
use brainiac_lib::agents::RunRuntime;
use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::db::{self, Db};
use brainiac_lib::models::{
    ErrorCode, HostSecretState, NamedSecret, SaveNamedSecretRequest, SecretSource,
};
use fake_engine::{fake_creds, FakeEngine};

const VALUE: &str = "sntrys_ExampleSentryTokenForTests0123456789";
const HOST: &str = "local";

struct Harness {
    tmp: tempfile::TempDir,
    store: Arc<MemoryStore>,
    named: NamedSecretService,
    runtime: RunRuntime,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new() -> Harness {
        Self::with_creds(None).await
    }

    /// `creds`: the sealing program the controller uses, or the stand-in.
    async fn with_creds(creds: Option<PathBuf>) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let core = Db::open(&tmp.path().join(db::CORE_FILE)).unwrap();
        let store = Arc::new(MemoryStore::default());
        let credentials = Arc::new(CredentialService::new(
            store.clone(),
            CommandRunner::new(tmp.path().join("commands")),
        ));
        let named = NamedSecretService::new(core, credentials);
        let state = StateDir::new(tmp.path().join("controller"));
        let socket = tmp.path().join("controller").join("runner.sock");
        let config = Config {
            state: state.clone(),
            socket: socket.clone(),
            idle_exit: None,
            spawn_guard: false,
            creds_program: Some(creds.unwrap_or_else(|| fake_creds(tmp.path()))),
            service: false,
        };
        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let server = tokio::spawn(async move {
            serve(config, engine).await.unwrap();
        });
        let runtime = RunRuntime::attach(state, socket);
        for _ in 0..100 {
            if runtime.running().await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Harness {
            tmp,
            store,
            named,
            runtime,
            server,
        }
    }

    fn request(
        &self,
        id: Option<&str>,
        version: i64,
        secret: Option<&str>,
    ) -> SaveNamedSecretRequest {
        SaveNamedSecretRequest {
            id: id.map(str::to_string),
            expected_version: version,
            label: "Sentry".into(),
            env_name: "SENTRY_TOKEN".into(),
            as_file: false,
            note: "Read-only, project X".into(),
            source: SecretSource::Store,
            secret: secret.map(str::to_string),
        }
    }

    async fn add(&self) -> NamedSecret {
        self.named
            .save(self.request(None, 0, Some(VALUE)))
            .await
            .unwrap()
    }

    async fn state(&self, id: &str) -> HostSecretState {
        let host = self
            .named
            .host_secrets(HOST, Some(&self.runtime))
            .await
            .unwrap();
        host.rows.iter().find(|r| r.secret_id == id).unwrap().state
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn a_secret_is_kept_in_the_keychain_and_shared_as_a_sealed_copy() {
    let h = Harness::new().await;
    let secret = h.add().await;
    assert_eq!(secret.credential_source, SecretSource::Store);
    assert!(h.store.text(&format!("named:{}", secret.id)).is_some());
    assert_eq!(h.state(&secret.id).await, HostSecretState::NotShared);

    let host = h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();
    let row = &host.rows[0];
    assert_eq!(row.state, HostSecretState::Shared);
    assert!(row.shared_at.is_some() && row.failure.is_none());
    assert!(host.only_on_host.is_empty() && host.problem.is_none());

    // The host holds it sealed, under the variable's name, and nothing lists the value.
    let sealed =
        std::fs::read_to_string(h.tmp.path().join("controller/secrets/SENTRY_TOKEN.cred")).unwrap();
    assert_ne!(sealed, VALUE);
    assert_eq!(
        h.runtime.list_secrets().await.unwrap()[0].name,
        "SENTRY_TOKEN"
    );
    assert!(!format!("{host:?}").contains(VALUE));
}

#[tokio::test]
async fn a_changed_secret_is_out_of_date_until_it_is_shared_again() {
    let h = Harness::new().await;
    let secret = h.add().await;
    h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();

    // A new value: the revision moves, and the host's copy is the old one.
    let saved = h
        .named
        .save(h.request(
            Some(&secret.id),
            secret.version,
            Some("a-rotated-sentry-token-value"),
        ))
        .await
        .unwrap();
    assert!(saved.credential.revision > secret.credential.revision);
    assert_eq!(h.state(&secret.id).await, HostSecretState::OutOfDate);

    h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();
    assert_eq!(h.state(&secret.id).await, HostSecretState::Shared);
    assert!(h.runtime.list_secrets().await.is_ok());
}

#[tokio::test]
async fn a_secret_the_host_lost_is_missing_and_one_only_the_host_holds_is_listed() {
    let h = Harness::new().await;
    let secret = h.add().await;
    h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();
    std::fs::remove_file(h.tmp.path().join("controller/secrets/SENTRY_TOKEN.cred")).unwrap();
    assert_eq!(h.state(&secret.id).await, HostSecretState::MissingOnHost);

    h.runtime
        .put_secret("OTHER_TOKEN", "12345678abcd".into(), false)
        .await
        .unwrap();
    let host = h.named.host_secrets(HOST, Some(&h.runtime)).await.unwrap();
    assert_eq!(host.only_on_host.len(), 1);
    assert_eq!(host.only_on_host[0].name, "OTHER_TOKEN");
}

#[tokio::test]
async fn a_host_that_cannot_seal_shows_the_share_as_failed_and_never_as_shared() {
    let tmp_missing = std::env::temp_dir().join("brainiac-no-such-creds-program");
    let h = Harness::with_creds(Some(tmp_missing)).await;
    let secret = h.add().await;
    let error = h
        .named
        .share(&secret.id, HOST, &h.runtime)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DependencyUnavailable);
    assert!(!format!("{error:?}").contains(VALUE));
    assert_eq!(h.state(&secret.id).await, HostSecretState::Failed);
    let host = h.named.host_secrets(HOST, Some(&h.runtime)).await.unwrap();
    assert!(host.rows[0]
        .failure
        .as_deref()
        .is_some_and(|f| f.contains("cannot keep secrets")));
}

#[tokio::test]
async fn a_share_that_never_reached_the_host_does_not_block_deleting() {
    let tmp_missing = std::env::temp_dir().join("brainiac-no-such-creds-program");
    let h = Harness::with_creds(Some(tmp_missing)).await;
    let secret = h.add().await;
    assert!(h.named.share(&secret.id, HOST, &h.runtime).await.is_err());
    h.named.delete(&secret.id, secret.version).await.unwrap();
    assert!(h.named.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_copy_no_secret_names_can_be_removed_by_name() {
    let h = Harness::new().await;
    let secret = h.add().await;
    h.runtime
        .put_secret("OTHER_TOKEN", "12345678abcd".into(), false)
        .await
        .unwrap();
    // A name a secret uses goes through its own Remove.
    h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();
    assert!(h
        .named
        .remove_unlisted("SENTRY_TOKEN", HOST, &h.runtime)
        .await
        .is_err());
    let host = h
        .named
        .remove_unlisted("OTHER_TOKEN", HOST, &h.runtime)
        .await
        .unwrap();
    assert!(host.only_on_host.is_empty());
}

#[tokio::test]
async fn removing_from_the_host_comes_before_deleting_or_renaming() {
    let h = Harness::new().await;
    let secret = h.add().await;
    h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();

    // A copy on a host would be left behind: refused.
    let error = h
        .named
        .delete(&secret.id, secret.version)
        .await
        .unwrap_err();
    assert!(
        error.message.contains("Remove it from the host"),
        "{error:?}"
    );
    let mut rename = h.request(Some(&secret.id), secret.version, None);
    rename.env_name = "SENTRY_AUTH".into();
    assert!(h.named.save(rename).await.is_err());

    let host = h.named.unshare(&secret.id, HOST, &h.runtime).await.unwrap();
    assert_eq!(host.rows[0].state, HostSecretState::NotShared);
    assert!(h.runtime.list_secrets().await.unwrap().is_empty());

    h.named.delete(&secret.id, secret.version).await.unwrap();
    assert!(h.named.list().await.unwrap().is_empty());
    assert!(h.store.text(&format!("named:{}", secret.id)).is_none());
}

#[tokio::test]
async fn names_values_and_sources_are_checked() {
    let h = Harness::new().await;
    for name in [
        "PATH",
        "NODE_OPTIONS",
        "sentry",
        "HTTPS_PROXY",
        "",
        "SSH_AUTH_SOCK",
        "RUBYOPT",
        "DOCKER_HOST",
    ] {
        let mut request = h.request(None, 0, Some(VALUE));
        request.env_name = name.into();
        assert!(h.named.save(request).await.is_err(), "{name:?}");
    }
    assert!(h.named.save(h.request(None, 0, None)).await.is_err());
    assert!(h
        .named
        .save(h.request(None, 0, Some("short")))
        .await
        .is_err());
    let mut ask = h.request(None, 0, None);
    ask.source = SecretSource::Ask;
    assert!(h.named.save(ask).await.is_err());
    assert!(h.named.list().await.unwrap().is_empty());

    // Two secrets cannot share a variable name.
    h.add().await;
    let error = h
        .named
        .save(h.request(None, 0, Some(VALUE)))
        .await
        .unwrap_err();
    assert!(error.message.contains("variable name"), "{error:?}");
    assert_eq!(h.named.list().await.unwrap().len(), 1);

    // A pasted value never reaches a log through the request.
    let text = format!("{:?}", h.request(None, 0, Some(VALUE)));
    assert!(!text.contains(VALUE), "{text}");
}

#[tokio::test]
async fn a_secret_for_a_file_reaches_the_host_as_one_and_cannot_change_while_there() {
    let h = Harness::new().await;
    let mut request = h.request(None, 0, Some(VALUE));
    request.env_name = "GCP_KEY".into();
    request.as_file = true;
    let secret = h.named.save(request).await.unwrap();
    assert!(secret.as_file);

    h.named.share(&secret.id, HOST, &h.runtime).await.unwrap();
    let listed = h.runtime.list_secrets().await.unwrap();
    assert!(listed.iter().any(|s| s.name == "GCP_KEY" && s.file));

    // The host holds it as a file: changing how a run gets it waits for Remove.
    let mut change = h.request(Some(&secret.id), secret.version, None);
    change.env_name = "GCP_KEY".into();
    change.as_file = false;
    let error = h.named.save(change).await.unwrap_err();
    assert!(
        error.message.contains("Remove it from the host"),
        "{error:?}"
    );
}
