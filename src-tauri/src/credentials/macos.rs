//! The macOS login keychain as Brainiac's store: generic passwords with
//! service `brainiac` and the owner's key as account.
//!
//! Reading an item another program created (such as `security`) makes macOS
//! ask the user to allow it, and the call waits for the answer, so these run
//! on a blocking thread (`CredentialService` does that).

use security_framework::base::Error as SecError;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords;

use super::store::SecretStore;
use super::{item_name, SecretBytes, SERVICE};
use crate::models::{AppError, AppResult, ErrorCode};

/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;
/// `errSecUserCanceled`, `errSecAuthFailed`, and `errSecInteractionNotAllowed`:
/// the user (or the system, for a locked keychain) refused access.
const REFUSED: [i32; 3] = [-128, -25293, -25308];

fn keychain_error(key: &str, action: &str, e: SecError) -> AppError {
    let code = if REFUSED.contains(&e.code()) {
        ErrorCode::PermissionDenied
    } else {
        ErrorCode::Io
    };
    AppError::new(
        code,
        format!(
            "Brainiac could not {action} the Keychain item {}.",
            item_name(key)
        ),
    )
    .with_details(e.to_string())
}

/// The login keychain of the user running Brainiac.
pub struct MacStore;

impl SecretStore for MacStore {
    fn contains(&self, key: &str) -> AppResult<bool> {
        let found = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .account(key)
            .load_attributes(true)
            .limit(Limit::Max(1))
            .search();
        match found {
            Ok(items) => Ok(!items.is_empty()),
            Err(e) if e.code() == NOT_FOUND => Ok(false),
            Err(e) => Err(keychain_error(key, "look for", e)),
        }
    }

    fn get(&self, key: &str) -> AppResult<Option<SecretBytes>> {
        match passwords::get_generic_password(SERVICE, key) {
            Ok(bytes) => Ok(Some(SecretBytes::new(bytes))),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(keychain_error(key, "read", e)),
        }
    }

    fn set(&self, key: &str, value: &SecretBytes) -> AppResult<()> {
        passwords::set_generic_password(SERVICE, key, value.expose())
            .map_err(|e| keychain_error(key, "save", e))
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        match passwords::delete_generic_password(SERVICE, key) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => Err(keychain_error(key, "delete", e)),
        }
    }

    fn name(&self) -> &'static str {
        "macOS login keychain"
    }
}
