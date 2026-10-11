//! The store Brainiac owns and writes: one item per owner, keyed as
//! `github`, `bitbucket`, or `db:<connection id>`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::SecretBytes;
use crate::models::{AppError, AppResult};

/// Where Brainiac keeps the secrets it owns. A trait so tests use memory
/// instead of the user's real Keychain; `Send + Sync` so one store can be
/// shared by the blocking threads that call it.
///
/// Calls may wait for the user to answer a system prompt, so they run on a
/// blocking thread, never on the async executor.
pub trait SecretStore: Send + Sync {
    /// Whether the item exists, found without reading it (no prompt).
    fn contains(&self, key: &str) -> AppResult<bool>;
    /// The item's bytes, or `None` when there is no such item.
    fn get(&self, key: &str) -> AppResult<Option<SecretBytes>>;
    /// Create the item, or replace what it holds.
    fn set(&self, key: &str, value: &SecretBytes) -> AppResult<()>;
    /// Delete the item; an item that is not there is not an error.
    fn delete(&self, key: &str) -> AppResult<()>;
    /// What the store is, for Settings → Secrets.
    fn name(&self) -> &'static str;
}

/// The store where the platform has none Brainiac supports yet (Linux, until
/// it has a Secret Service store; docs/design/linux.md). Nothing is ever in
/// it, and saving to it is refused rather than written anywhere else, so a
/// secret never lands in a file; an environment variable or a command still
/// works as a source.
pub struct NoStore;

impl NoStore {
    fn refused() -> AppError {
        AppError::dependency(
            "This system has no secret store Brainiac can use. Choose an environment variable or a command as the source.",
        )
    }
}

impl SecretStore for NoStore {
    fn contains(&self, _key: &str) -> AppResult<bool> {
        Ok(false)
    }
    fn get(&self, _key: &str) -> AppResult<Option<SecretBytes>> {
        Err(Self::refused())
    }
    fn set(&self, _key: &str, _value: &SecretBytes) -> AppResult<()> {
        Err(Self::refused())
    }
    fn delete(&self, _key: &str) -> AppResult<()> {
        // Nothing was ever saved, so there is nothing to delete.
        Ok(())
    }
    fn name(&self) -> &'static str {
        "No secret store"
    }
}

/// Items in memory, for tests, with switches that make writes fail and a
/// count of reads.
#[derive(Default)]
pub struct MemoryStore {
    // `Mutex` because the trait's methods take `&self` and may run on any thread.
    items: Mutex<HashMap<String, Vec<u8>>>,
    reads: AtomicUsize,
    fail_set: AtomicBool,
    fail_delete: AtomicBool,
}

impl MemoryStore {
    /// How many times `get` was called.
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    /// Make every `set` fail until turned off again.
    pub fn fail_writes(&self, fail: bool) {
        self.fail_set.store(fail, Ordering::SeqCst);
    }

    /// Make every `delete` fail until turned off again.
    pub fn fail_deletes(&self, fail: bool) {
        self.fail_delete.store(fail, Ordering::SeqCst);
    }

    /// The item as text, for assertions.
    pub fn text(&self, key: &str) -> Option<String> {
        self.items
            .lock()
            .expect("store lock")
            .get(key)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// Put an item in place, as `security` from Terminal would.
    pub fn insert(&self, key: &str, text: &str) {
        self.items
            .lock()
            .expect("store lock")
            .insert(key.to_string(), text.as_bytes().to_vec());
    }
}

impl SecretStore for MemoryStore {
    fn contains(&self, key: &str) -> AppResult<bool> {
        Ok(self.items.lock().expect("store lock").contains_key(key))
    }

    fn get(&self, key: &str) -> AppResult<Option<SecretBytes>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .items
            .lock()
            .expect("store lock")
            .get(key)
            .map(|b| SecretBytes::new(b.clone())))
    }

    fn set(&self, key: &str, value: &SecretBytes) -> AppResult<()> {
        if self.fail_set.load(Ordering::SeqCst) {
            return Err(AppError::io("The test store refused the write."));
        }
        self.items
            .lock()
            .expect("store lock")
            .insert(key.to_string(), value.expose().to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(AppError::io("The test store refused the deletion."));
        }
        self.items.lock().expect("store lock").remove(key);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "Memory (tests)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ErrorCode;

    #[test]
    fn no_store_holds_nothing_and_refuses_to_save() {
        let store = NoStore;
        assert!(!store.contains("github").unwrap());
        let value = SecretBytes::new(b"token".to_vec());
        let refused = store.set("github", &value).unwrap_err();
        assert_eq!(refused.code, ErrorCode::DependencyUnavailable);
        assert!(refused.message.contains("environment variable"));
        assert_eq!(
            store.get("github").unwrap_err().code,
            ErrorCode::DependencyUnavailable
        );
        store.delete("github").unwrap();
    }
}
