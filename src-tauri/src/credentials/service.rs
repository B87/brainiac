//! Resolving a binding to a secret, once per run (SPEC.md, Secrets;
//! docs/architecture.md, Credentials).
//!
//! An owner (`github`, `db:<id>`) has one active binding: its source, the
//! revision of that source and of where the secret is sent, and whether the
//! user approved it. Domain services build bindings from their rows and own
//! every rule about them; this service owns reading, the cache, and the
//! per-owner gate that serializes credential changes.
//!
//! Each owner's state has a *generation*, advanced whenever its cache is
//! invalidated (a refresh, a save, a refusal). A read remembers the
//! generation it started in; one that finishes after the generation moved on
//! is obsolete and installs nothing, and a refusal of an old lease cannot
//! evict its replacement.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use tokio::sync::watch;

use super::command::CommandRunner;
use super::store::SecretStore;
use super::{item_name, sources, SecretBytes, MAX_SECRET_BYTES};
use crate::models::{
    AppError, AppResult, CredentialPending, CredentialTest, ErrorCode, SecretSource,
};

/// An owner's credential as its row describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The owner's key, which is also the name of its store item.
    pub owner: String,
    pub source: SecretSource,
    /// Advanced by the domain whenever the source, where the secret is
    /// sent, or the stored secret changes.
    pub revision: i64,
    /// False after a restore until the user confirms the source.
    pub approved: bool,
    pub pending: Option<CredentialPending>,
    /// A known expiry, such as an account token's: a cached value is not used after it.
    pub expires_at: Option<DateTime<Utc>>,
}

/// A resolved secret and the revision and generation it was read for. It
/// never crosses IPC; its `Debug` prints nothing of the value.
#[derive(Clone)]
pub struct Lease {
    owner: String,
    revision: i64,
    generation: u64,
    // `Arc` so callers sharing one read share one copy of the bytes.
    value: Arc<SecretBytes>,
}

impl Lease {
    pub fn bytes(&self) -> &SecretBytes {
        &self.value
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn revision(&self) -> i64 {
        self.revision
    }

    /// Whether both came from the same read of the same binding.
    pub fn same_read(&self, other: &Lease) -> bool {
        self.owner == other.owner
            && self.revision == other.revision
            && self.generation == other.generation
    }
}

impl fmt::Debug for Lease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lease")
            .field("owner", &self.owner)
            .field("revision", &self.revision)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// A lease with the service that issued it, for code far from the domain
/// service (an HTTP adapter) that must report a refusal.
#[derive(Clone)]
pub struct LeaseHandle {
    service: Arc<CredentialService>,
    lease: Lease,
}

impl fmt::Debug for LeaseHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.lease.fmt(f)
    }
}

impl LeaseHandle {
    pub fn new(service: Arc<CredentialService>, lease: Lease) -> Self {
        LeaseHandle { service, lease }
    }

    pub fn lease(&self) -> &Lease {
        &self.lease
    }

    /// The server refused this secret: the next use reads it again.
    pub fn reject(&self) {
        self.service.reject(&self.lease);
    }

    pub fn is_current(&self) -> bool {
        self.service.is_current(&self.lease)
    }
}

/// What resolving a binding gave.
#[derive(Debug)]
pub enum Resolution {
    Secret(Lease),
    /// The source is None: connect without a secret.
    NoCredential,
    /// The source is Ask and nothing was typed for this revision yet.
    InputRequired,
}

/// What reading a draft's source gave, for Test. Never cached.
#[derive(Debug)]
pub enum Probe {
    Secret(SecretBytes),
    NoCredential,
    InputRequired,
}

/// Held while an owner's credential changes: saves, removals, cleanups, and
/// approvals of the same owner run one at a time. Store writes need it, so
/// only the owner's own item can be written.
pub struct OwnerGate {
    owner: String,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl OwnerGate {
    pub fn owner(&self) -> &str {
        &self.owner
    }
}

/// The result a read publishes to everyone waiting for it.
type ReadResult = Option<Result<Arc<SecretBytes>, AppError>>;

struct Cached {
    revision: i64,
    generation: u64,
    value: Arc<SecretBytes>,
    expires_at: Option<DateTime<Utc>>,
}

struct Reading {
    generation: u64,
    done: watch::Receiver<ReadResult>,
}

#[derive(Default)]
struct OwnerState {
    /// The newest revision seen for this owner.
    revision: i64,
    /// The owner was removed at this revision: bindings at or below it are
    /// gone, so a read that started before the removal cannot run its source.
    removed: i64,
    generation: u64,
    /// A save or removal is under way: nothing resolves until it commits.
    blocked: bool,
    cached: Option<Cached>,
    reading: Option<Reading>,
    /// Ask: what was typed, and for which revision.
    typed: Option<(i64, Arc<SecretBytes>)>,
    last_test: Option<(i64, CredentialTest)>,
    // `Arc<tokio::sync::Mutex<()>>`: an async lock, held across awaits by
    // `OwnerGate`, and shared so the map's own lock is never held while waiting.
    gate: Arc<tokio::sync::Mutex<()>>,
}

impl OwnerState {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.cached = None;
        self.reading = None;
        self.typed = None;
    }

    fn lease(&self, owner: &str, revision: i64, value: &Arc<SecretBytes>) -> Lease {
        Lease {
            owner: owner.to_string(),
            revision,
            generation: self.generation,
            value: Arc::clone(value),
        }
    }
}

// `Arc<Mutex<_>>`: the state is shared with the tasks that run reads, and
// the lock is held only for short bookkeeping, never across an await.
type Owners = Arc<Mutex<HashMap<String, OwnerState>>>;

enum Step {
    Done(Resolution),
    Wait {
        generation: u64,
        done: watch::Receiver<ReadResult>,
    },
}

pub struct CredentialService {
    // `Arc<dyn SecretStore>`: the Keychain in the app, memory in tests,
    // shared with the blocking threads that call it.
    store: Arc<dyn SecretStore>,
    runner: CommandRunner,
    owners: Owners,
}

fn removed() -> AppError {
    AppError::not_found("This was removed while its secret was being read.")
}

fn stale() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "This secret's source changed while it was being used. Try again.",
    )
}

/// Refuse a binding that is blocked or not approved, before anything is read.
fn check_binding(binding: &Binding) -> AppResult<()> {
    match binding.pending {
        Some(CredentialPending::Save) => {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "The last save of this secret did not finish, so Brainiac does not know what its Keychain item holds. Edit it, enter the secret again or choose another source, and save.",
            ))
        }
        Some(CredentialPending::Removal) => {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "This is being removed, and its Keychain item is not deleted yet. Retry the removal in Settings → Secrets.",
            ))
        }
        Some(CredentialPending::Cleanup) | None => {}
    }
    if !binding.approved {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "This secret's source came from a restored backup and is not read until you allow it. Check it in Settings → Secrets.",
        ));
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        AppError::io("The Keychain call did not finish.").with_details(e.to_string())
    })?
}

/// Read a source afresh. Ask and None have nothing to read.
async fn read_source(
    store: Arc<dyn SecretStore>,
    runner: &CommandRunner,
    owner: &str,
    source: &SecretSource,
) -> AppResult<SecretBytes> {
    let value = match source {
        SecretSource::Store => {
            let key = owner.to_string();
            blocking(move || store.get(&key)).await?.ok_or_else(|| {
                AppError::not_found(format!("The Keychain has no item {}.", item_name(owner)))
            })?
        }
        SecretSource::Environment { name } => sources::environment(name)?,
        SecretSource::Command { program, args } => runner.run(program, args).await?,
        SecretSource::Ask | SecretSource::None => {
            return Err(AppError::validation("This source has nothing to read."))
        }
    };
    if value.len() > MAX_SECRET_BYTES {
        return Err(AppError::validation(format!(
            "{} holds more than {} KiB.",
            sources::describe(source, owner),
            MAX_SECRET_BYTES / 1024
        )));
    }
    Ok(value)
}

fn expired(at: Option<DateTime<Utc>>) -> bool {
    at.is_some_and(|at| Utc::now() >= at)
}

impl CredentialService {
    pub fn new(store: Arc<dyn SecretStore>, runner: CommandRunner) -> Self {
        CredentialService {
            store,
            runner,
            owners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// What the store is, for Settings → Secrets.
    pub fn store_name(&self) -> &'static str {
        self.store.name()
    }

    /// The binding's secret: cached for this run, or read once however many
    /// callers ask at the same time. Missing values and failures are not
    /// cached, so the next use reads again.
    pub async fn resolve(&self, binding: &Binding) -> AppResult<Resolution> {
        // A read that became obsolete while waited for is retried a few times.
        for _ in 0..3 {
            let (generation, mut done) = match self.step(binding)? {
                Step::Done(resolution) => return Ok(resolution),
                Step::Wait { generation, done } => (generation, done),
            };
            // `wait_for` fails only when the reading task ended without
            // publishing, which it does only by panicking.
            let result = done
                .wait_for(Option::is_some)
                .await
                .ok()
                .and_then(|r| r.clone());
            let current = self
                .owners
                .lock()
                .expect("credentials lock")
                .get(&binding.owner)
                .is_some_and(|s| {
                    s.generation == generation && s.revision == binding.revision && !s.blocked
                });
            if !current {
                continue;
            }
            return match result {
                Some(Ok(value)) => Ok(Resolution::Secret(Lease {
                    owner: binding.owner.clone(),
                    revision: binding.revision,
                    generation,
                    value,
                })),
                Some(Err(e)) => Err(e),
                None => Err(AppError::io("Reading the secret did not finish.")),
            };
        }
        Err(stale())
    }

    fn step(&self, binding: &Binding) -> AppResult<Step> {
        check_binding(binding)?;
        let mut owners = self.owners.lock().expect("credentials lock");
        let state = owners.entry(binding.owner.clone()).or_default();
        if binding.revision <= state.removed {
            return Err(removed());
        }
        if binding.revision < state.revision {
            return Err(stale());
        }
        if binding.revision > state.revision {
            state.revision = binding.revision;
            state.invalidate();
        }
        if state.blocked {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This secret is being saved. Try again in a moment.",
            ));
        }
        match &binding.source {
            SecretSource::None => return Ok(Step::Done(Resolution::NoCredential)),
            SecretSource::Ask => {
                return Ok(Step::Done(match &state.typed {
                    Some((revision, value)) if *revision == binding.revision => {
                        Resolution::Secret(state.lease(&binding.owner, binding.revision, value))
                    }
                    _ => Resolution::InputRequired,
                }))
            }
            _ => {}
        }
        if let Some(cached) = &state.cached {
            if cached.revision == binding.revision
                && cached.generation == state.generation
                && !expired(cached.expires_at)
            {
                let lease = state.lease(&binding.owner, binding.revision, &cached.value);
                return Ok(Step::Done(Resolution::Secret(lease)));
            }
            // Expired or obsolete: a new generation, so the read that
            // replaces it is a new lease, checked again by its domain, and a
            // late refusal of the old one cannot evict it.
            state.invalidate();
        }
        if let Some(reading) = &state.reading {
            if reading.generation == state.generation {
                return Ok(Step::Wait {
                    generation: reading.generation,
                    done: reading.done.clone(),
                });
            }
        }

        // Nobody is reading this generation yet: start a read on its own
        // task, so a caller that stops waiting does not stop it, and a second
        // caller joins it instead of causing a second Keychain prompt.
        let generation = state.generation;
        let (tx, rx) = watch::channel(None);
        state.reading = Some(Reading {
            generation,
            done: rx.clone(),
        });
        let store = Arc::clone(&self.store);
        let runner = self.runner.clone();
        let owners_handle = Arc::clone(&self.owners);
        let owner = binding.owner.clone();
        let source = binding.source.clone();
        let revision = binding.revision;
        let expires_at = binding.expires_at;
        tokio::spawn(async move {
            let result = read_source(store, &runner, &owner, &source)
                .await
                .map(Arc::new);
            {
                let mut owners = owners_handle.lock().expect("credentials lock");
                if let Some(state) = owners.get_mut(&owner) {
                    if state
                        .reading
                        .as_ref()
                        .is_some_and(|r| r.generation == generation)
                    {
                        state.reading = None;
                    }
                    if let Ok(value) = &result {
                        if state.generation == generation
                            && state.revision == revision
                            && !state.blocked
                        {
                            state.cached = Some(Cached {
                                revision,
                                generation,
                                value: Arc::clone(value),
                                expires_at,
                            });
                        }
                    }
                }
            }
            let _ = tx.send(Some(result));
        });
        Ok(Step::Wait {
            generation,
            done: rx,
        })
    }

    /// Read a draft's source afresh, for Test: nothing is cached or
    /// invalidated, and no binding changes. Approval is the domain's check.
    pub async fn probe(&self, owner: &str, source: &SecretSource) -> AppResult<Probe> {
        match source {
            SecretSource::None => Ok(Probe::NoCredential),
            SecretSource::Ask => Ok(Probe::InputRequired),
            source => Ok(Probe::Secret(
                read_source(Arc::clone(&self.store), &self.runner, owner, source).await?,
            )),
        }
    }

    /// Keep what was typed for an Ask binding, for its current revision.
    pub fn supply(&self, binding: &Binding, value: SecretBytes) -> AppResult<()> {
        check_binding(binding)?;
        if binding.source != SecretSource::Ask {
            return Err(AppError::validation(
                "This connection does not ask for its password.",
            ));
        }
        let mut owners = self.owners.lock().expect("credentials lock");
        let state = owners.entry(binding.owner.clone()).or_default();
        if binding.revision <= state.removed {
            return Err(removed());
        }
        if binding.revision < state.revision {
            return Err(stale());
        }
        if binding.revision > state.revision {
            state.revision = binding.revision;
            state.invalidate();
        }
        if state.blocked {
            return Err(stale());
        }
        state.typed = Some((binding.revision, Arc::new(value)));
        Ok(())
    }

    /// Whether something was typed for this Ask revision.
    pub fn typed_ready(&self, owner: &str, revision: i64) -> bool {
        self.owners
            .lock()
            .expect("credentials lock")
            .get(owner)
            .is_some_and(|s| matches!(&s.typed, Some((r, _)) if *r == revision))
    }

    /// A server refused this lease: forget it, so the next use reads (or
    /// asks) again. A lease already replaced is left alone. The source
    /// itself is never told to erase anything.
    pub fn reject(&self, lease: &Lease) {
        let mut owners = self.owners.lock().expect("credentials lock");
        if let Some(state) = owners.get_mut(&lease.owner) {
            if state.revision == lease.revision && state.generation == lease.generation {
                state.invalidate();
                tracing::info!(owner = %lease.owner, "a credential was refused and forgotten");
            }
        }
    }

    /// Whether the lease is still the owner's current credential.
    pub fn is_current(&self, lease: &Lease) -> bool {
        self.owners
            .lock()
            .expect("credentials lock")
            .get(&lease.owner)
            .is_some_and(|s| {
                s.revision == lease.revision && s.generation == lease.generation && !s.blocked
            })
    }

    /// Refresh credential: forget the cached value and anything typed, so
    /// the next use reads or asks again. Reads under way become obsolete.
    pub fn invalidate(&self, owner: &str) {
        if let Some(state) = self.owners.lock().expect("credentials lock").get_mut(owner) {
            state.invalidate();
        }
    }

    /// Wait for the owner's gate.
    pub async fn gate(&self, owner: &str) -> OwnerGate {
        let gate = Arc::clone(
            &self
                .owners
                .lock()
                .expect("credentials lock")
                .entry(owner.to_string())
                .or_default()
                .gate,
        );
        OwnerGate {
            owner: owner.to_string(),
            _guard: gate.lock_owned().await,
        }
    }

    fn with_state<T>(&self, owner: &str, f: impl FnOnce(&mut OwnerState) -> T) -> T {
        f(self
            .owners
            .lock()
            .expect("credentials lock")
            .entry(owner.to_string())
            .or_default())
    }

    /// A pending marker was committed: nothing resolves until `commit`, and
    /// reads under way become obsolete.
    pub fn begin(&self, gate: &OwnerGate) {
        self.with_state(&gate.owner, |s| {
            s.blocked = true;
            s.invalidate();
        });
    }

    /// A binding change was committed at `revision`: earlier reads and
    /// leases are obsolete, and resolving works again.
    pub fn commit(&self, gate: &OwnerGate, revision: i64) {
        self.with_state(&gate.owner, |s| {
            s.revision = revision;
            s.blocked = false;
            s.invalidate();
        });
    }

    /// After a successful commit, keep the value just checked or written, so
    /// using it does not read it again.
    pub fn prime(
        &self,
        gate: &OwnerGate,
        revision: i64,
        value: SecretBytes,
        expires_at: Option<DateTime<Utc>>,
    ) -> Option<Lease> {
        self.with_state(&gate.owner, |s| {
            if s.revision != revision || s.blocked {
                return None;
            }
            let value = Arc::new(value);
            s.cached = Some(Cached {
                revision,
                generation: s.generation,
                value: Arc::clone(&value),
                expires_at,
            });
            Some(s.lease(&gate.owner, revision, &value))
        })
    }

    /// The owner is gone: forget everything about it except its gate.
    ///
    /// Its revision is kept: an owner added again under the same key (an
    /// account of the same provider) starts above it (`next_revision`).
    pub fn forget(&self, gate: &OwnerGate) {
        self.with_state(&gate.owner, |s| {
            s.invalidate();
            s.removed = s.removed.max(s.revision);
            s.blocked = false;
            s.last_test = None;
        });
    }

    /// The revision a new binding of this owner starts at: above every
    /// revision seen in this run, including a removed owner's.
    pub fn next_revision(&self, owner: &str) -> i64 {
        self.owners
            .lock()
            .expect("credentials lock")
            .get(owner)
            .map_or(1, |s| s.revision.max(s.removed) + 1)
    }

    /// Create or replace the owner's store item.
    pub async fn write_item(&self, gate: &OwnerGate, value: SecretBytes) -> AppResult<()> {
        let (store, key) = (Arc::clone(&self.store), gate.owner.clone());
        blocking(move || store.set(&key, &value)).await
    }

    /// Delete the owner's store item; one that is not there is not an error.
    pub async fn delete_item(&self, gate: &OwnerGate) -> AppResult<()> {
        let (store, key) = (Arc::clone(&self.store), gate.owner.clone());
        blocking(move || store.delete(&key)).await
    }

    /// Whether the owner's store item exists, found without reading it.
    pub async fn item_exists(&self, owner: &str) -> AppResult<bool> {
        let (store, key) = (Arc::clone(&self.store), owner.to_string());
        blocking(move || store.contains(&key)).await
    }

    /// Remember a Test of the saved binding at `revision`, for Settings → Secrets.
    pub fn record_test(&self, owner: &str, revision: i64, test: CredentialTest) {
        self.with_state(owner, |s| s.last_test = Some((revision, test)));
    }

    /// The last Test of this revision, if any in this run.
    pub fn last_test(&self, owner: &str, revision: i64) -> Option<CredentialTest> {
        self.owners
            .lock()
            .expect("credentials lock")
            .get(owner)
            .and_then(|s| s.last_test.as_ref())
            .filter(|(r, _)| *r == revision)
            .map(|(_, t)| t.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::credentials::MemoryStore;

    /// A store whose reads wait until released, counting them.
    #[derive(Default)]
    struct SlowStore {
        reads: AtomicUsize,
        // A std `Mutex` and `Condvar`: reads run on blocking threads.
        open: std::sync::Mutex<bool>,
        opened: std::sync::Condvar,
        value: std::sync::Mutex<String>,
    }

    impl SlowStore {
        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.opened.notify_all();
        }
    }

    impl SecretStore for SlowStore {
        fn contains(&self, _: &str) -> AppResult<bool> {
            Ok(true)
        }
        fn get(&self, _: &str) -> AppResult<Option<SecretBytes>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
            Ok(Some(SecretBytes::from_text(&self.value.lock().unwrap())))
        }
        fn set(&self, _: &str, _: &SecretBytes) -> AppResult<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        fn name(&self) -> &'static str {
            "slow"
        }
    }

    fn service(store: Arc<dyn SecretStore>) -> Arc<CredentialService> {
        let dir = std::env::temp_dir().join("brainiac-credential-tests");
        Arc::new(CredentialService::new(store, CommandRunner::new(dir)))
    }

    fn store_binding(revision: i64) -> Binding {
        Binding {
            owner: "db:1".into(),
            source: SecretSource::Store,
            revision,
            approved: true,
            pending: None,
            expires_at: None,
        }
    }

    fn secret(r: Resolution) -> Lease {
        match r {
            Resolution::Secret(lease) => lease,
            other => panic!("not a secret: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_reads_share_one_store_access() {
        let store = Arc::new(SlowStore::default());
        *store.value.lock().unwrap() = "pw".into();
        let credentials = service(store.clone());
        let binding = store_binding(1);
        let (a, b) = (Arc::clone(&credentials), Arc::clone(&credentials));
        let (ba, bb) = (binding.clone(), binding.clone());
        let first = tokio::spawn(async move { a.resolve(&ba).await });
        let second = tokio::spawn(async move { b.resolve(&bb).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        store.release();
        let first = secret(first.await.unwrap().unwrap());
        let second = secret(second.await.unwrap().unwrap());
        assert_eq!(first.bytes().expose(), b"pw");
        assert_eq!(second.bytes().expose(), b"pw");
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
        // Cached for the run.
        secret(credentials.resolve(&binding).await.unwrap());
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_waiter_that_gives_up_does_not_cause_a_second_read() {
        let store = Arc::new(SlowStore::default());
        *store.value.lock().unwrap() = "pw".into();
        let credentials = service(store.clone());
        let binding = store_binding(1);
        let gave_up =
            tokio::time::timeout(Duration::from_millis(50), credentials.resolve(&binding)).await;
        assert!(gave_up.is_err());
        let again = {
            let (c, b) = (Arc::clone(&credentials), binding.clone());
            tokio::spawn(async move { c.resolve(&b).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        store.release();
        secret(again.await.unwrap().unwrap());
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_read_finishing_after_a_refresh_installs_nothing() {
        let store = Arc::new(SlowStore::default());
        *store.value.lock().unwrap() = "old".into();
        let credentials = service(store.clone());
        let binding = store_binding(1);
        let waiting = {
            let (c, b) = (Arc::clone(&credentials), binding.clone());
            tokio::spawn(async move { c.resolve(&b).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        credentials.invalidate("db:1");
        *store.value.lock().unwrap() = "new".into();
        store.release();
        // The waiter retried with a fresh read rather than taking the obsolete one.
        let lease = secret(waiting.await.unwrap().unwrap());
        assert_eq!(lease.bytes().expose(), b"new");
        assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_old_lease_refused_late_does_not_evict_its_replacement() {
        let store = Arc::new(MemoryStore::default());
        store.insert("db:1", "first");
        let credentials = service(store.clone());
        let binding = store_binding(1);
        let old = secret(credentials.resolve(&binding).await.unwrap());
        credentials.reject(&old);
        assert!(!credentials.is_current(&old));
        store.insert("db:1", "second");
        let new = secret(credentials.resolve(&binding).await.unwrap());
        assert_eq!(new.bytes().expose(), b"second");
        credentials.reject(&old);
        assert!(credentials.is_current(&new));
        secret(credentials.resolve(&binding).await.unwrap());
        assert_eq!(store.reads(), 2);
    }

    #[tokio::test]
    async fn missing_values_are_not_cached_and_blocked_bindings_never_read() {
        let store = Arc::new(MemoryStore::default());
        let credentials = service(store.clone());
        let mut binding = store_binding(1);
        assert_eq!(
            credentials.resolve(&binding).await.unwrap_err().code,
            ErrorCode::NotFound
        );
        store.insert("db:1", "pw");
        secret(credentials.resolve(&binding).await.unwrap());
        assert_eq!(store.reads(), 2);

        binding.approved = false;
        assert_eq!(
            credentials.resolve(&binding).await.unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        binding.approved = true;
        binding.pending = Some(CredentialPending::Save);
        assert!(credentials.resolve(&binding).await.is_err());
        binding.pending = Some(CredentialPending::Cleanup);
        secret(credentials.resolve(&binding).await.unwrap());
        assert_eq!(store.reads(), 2, "a cache hit still checks approval first");

        // An older revision than the one committed is refused.
        let gate = credentials.gate("db:1").await;
        credentials.begin(&gate);
        assert!(credentials.resolve(&store_binding(1)).await.is_err());
        credentials.commit(&gate, 2);
        drop(gate);
        assert_eq!(
            credentials
                .resolve(&store_binding(1))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        secret(credentials.resolve(&store_binding(2)).await.unwrap());
    }

    #[tokio::test]
    async fn ask_and_none_are_not_failed_reads() {
        let credentials = service(Arc::new(MemoryStore::default()));
        let none = Binding {
            source: SecretSource::None,
            ..store_binding(1)
        };
        assert!(matches!(
            credentials.resolve(&none).await.unwrap(),
            Resolution::NoCredential
        ));
        let ask = Binding {
            source: SecretSource::Ask,
            ..store_binding(1)
        };
        assert!(matches!(
            credentials.resolve(&ask).await.unwrap(),
            Resolution::InputRequired
        ));
        credentials
            .supply(&ask, SecretBytes::from_text("typed"))
            .unwrap();
        assert!(credentials.typed_ready("db:1", 1));
        let lease = secret(credentials.resolve(&ask).await.unwrap());
        credentials.reject(&lease);
        assert!(matches!(
            credentials.resolve(&ask).await.unwrap(),
            Resolution::InputRequired
        ));
        // Typed for an older revision: refused once a newer one is known.
        let newer = Binding {
            revision: 2,
            ..ask.clone()
        };
        credentials.resolve(&newer).await.unwrap();
        assert!(credentials
            .supply(&ask, SecretBytes::from_text("stale"))
            .is_err());
    }

    #[tokio::test]
    async fn a_removed_owner_is_not_read_by_an_old_binding_and_restarts_above_it() {
        let store = Arc::new(MemoryStore::default());
        store.insert("github", "old");
        let credentials = service(store.clone());
        let old = Binding {
            owner: "github".into(),
            ..store_binding(5)
        };
        secret(credentials.resolve(&old).await.unwrap());
        let gate = credentials.gate("github").await;
        credentials.forget(&gate);
        drop(gate);
        // A read that started before the removal finds it gone.
        assert_eq!(
            credentials.resolve(&old).await.unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(store.reads(), 1);
        // The account added again starts above the removed one.
        let next = credentials.next_revision("github");
        assert_eq!(next, 6);
        let new = Binding {
            revision: next,
            ..old.clone()
        };
        secret(credentials.resolve(&new).await.unwrap());
        assert_eq!(credentials.next_revision("db:never-seen"), 1);
    }

    #[tokio::test]
    async fn expired_values_are_read_again() {
        let store = Arc::new(MemoryStore::default());
        store.insert("db:1", "pw");
        let credentials = service(store.clone());
        let binding = Binding {
            expires_at: Some(Utc::now() - chrono::Duration::seconds(1)),
            ..store_binding(1)
        };
        let first = secret(credentials.resolve(&binding).await.unwrap());
        let second = secret(credentials.resolve(&binding).await.unwrap());
        assert_eq!(store.reads(), 2);
        // Each read is a lease of its own: checked again, and a refusal of
        // the first does not evict the second.
        assert!(!first.same_read(&second));
        credentials.reject(&first);
        assert!(credentials.is_current(&second));
    }
}
