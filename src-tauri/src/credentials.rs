//! Secrets in the macOS Keychain, shared by pull request accounts and
//! database connections (docs/architecture.md, Databases — v0.4, Credentials).
//!
//! Every item is a generic password with service `brainiac` and an account
//! that names its owner: `github`, `bitbucket`, or `db:<connection id>`.
//!
//! Reading an item another program created (such as `security`) makes macOS
//! ask the user to allow it, and the call waits for the answer, so callers run
//! these on a blocking thread, never on the async executor.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use security_framework::base::Error as SecError;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords;

use crate::models::{AppError, AppResult, ErrorCode};

/// The Keychain service name of every item.
pub const SERVICE: &str = "brainiac";

/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;
/// `errSecUserCanceled`, `errSecAuthFailed`, and `errSecInteractionNotAllowed`:
/// the user (or the system, for a locked keychain) refused access.
const REFUSED: [i32; 3] = [-128, -25293, -25308];

fn item_name(account: &str) -> String {
    format!("{SERVICE}/{account}")
}

fn keychain_error(account: &str, action: &str, e: SecError) -> AppError {
    let code = if REFUSED.contains(&e.code()) {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::Io
    };
    AppError::new(
        code,
        format!(
            "Brainiac could not {action} the Keychain item {}.",
            item_name(account)
        ),
    )
    .with_details(e.to_string())
}

/// Whether the item exists, found without reading it (no prompt).
pub fn contains(account: &str) -> AppResult<bool> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .account(account)
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search();
    match found {
        Ok(items) => Ok(!items.is_empty()),
        Err(e) if e.code() == NOT_FOUND => Ok(false),
        Err(e) => Err(keychain_error(account, "look for", e)),
    }
}

/// The item's bytes, or `None` when there is no such item.
pub fn read(account: &str) -> AppResult<Option<Vec<u8>>> {
    match passwords::get_generic_password(SERVICE, account) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(keychain_error(account, "read", e)),
    }
}

/// Create the item, or replace what it holds.
pub fn write(account: &str, bytes: &[u8]) -> AppResult<()> {
    passwords::set_generic_password(SERVICE, account, bytes)
        .map_err(|e| keychain_error(account, "save", e))
}

/// Delete the item; an item that is not there is not an error.
pub fn remove(account: &str) -> AppResult<()> {
    match passwords::delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => Ok(()),
        Err(e) => Err(keychain_error(account, "delete", e)),
    }
}

/// A password. It has no `Display` and its `Debug` prints nothing of it, so
/// it cannot end up in a message or a log by accident; `expose` is called
/// only where it goes to a server or the Keychain.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// A password as typed: kept exactly, spaces included, but never empty.
    pub fn new(text: &str) -> AppResult<Self> {
        if text.is_empty() {
            return Err(AppError::validation("Enter the password."));
        }
        Ok(Secret(text.to_string()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Where passwords are kept, by Keychain account. A trait so tests can use
/// memory instead of the user's real Keychain; `Send + Sync` lets one store
/// be shared by threads.
pub trait Secrets: Send + Sync {
    fn get(&self, account: &str) -> AppResult<Option<Secret>>;
    fn set(&self, account: &str, secret: &Secret) -> AppResult<()>;
    fn delete(&self, account: &str) -> AppResult<()>;
}

/// The login keychain of the user running Brainiac.
pub struct MacSecrets;

impl Secrets for MacSecrets {
    fn get(&self, account: &str) -> AppResult<Option<Secret>> {
        let Some(bytes) = read(account)? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes).map_err(|_| {
            AppError::validation(format!(
                "The Keychain item {} does not hold a password.",
                item_name(account)
            ))
        })?;
        Secret::new(&text).map(Some)
    }

    fn set(&self, account: &str, secret: &Secret) -> AppResult<()> {
        write(account, secret.expose().as_bytes())
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        remove(account)
    }
}

/// Passwords in memory, for tests.
#[derive(Default)]
pub struct MemorySecrets {
    // `Mutex` because the trait's methods take `&self` and may run on any thread.
    items: Mutex<HashMap<String, Secret>>,
}

impl Secrets for MemorySecrets {
    fn get(&self, account: &str) -> AppResult<Option<Secret>> {
        Ok(self
            .items
            .lock()
            .expect("secrets lock")
            .get(account)
            .cloned())
    }

    fn set(&self, account: &str, secret: &Secret) -> AppResult<()> {
        self.items
            .lock()
            .expect("secrets lock")
            .insert(account.to_string(), secret.clone());
        Ok(())
    }

    fn delete(&self, account: &str) -> AppResult<()> {
        self.items.lock().expect("secrets lock").remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_keep_spaces_and_are_never_printed() {
        let secret = Secret::new(" pass word ").unwrap();
        assert_eq!(secret.expose(), " pass word ");
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert!(Secret::new("").is_err());
    }
}
