//! Account tokens in the macOS Keychain (SPEC.md, Accounts): a generic
//! password with service `brainiac` and account `github` or `bitbucket`, a
//! fixed name so a token can also be added from Terminal with `security`.
//!
//! The Keychain calls themselves are shared with database passwords
//! (`credentials.rs`); like them, these run on a blocking thread.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use crate::credentials;
use crate::models::{AppError, AppResult, ForgeKind};

/// The Keychain service name of every account token.
pub use crate::credentials::SERVICE;

/// A token. It has no `Display` and its `Debug` prints nothing of it, so it
/// cannot end up in a message or a log by accident; `expose` is called only
/// where it goes into a request header or the Keychain.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A token from pasted text: surrounding whitespace removed, never empty.
    pub fn new(text: &str) -> AppResult<Self> {
        let text = text.trim();
        if text.is_empty() {
            return Err(AppError::validation("Paste a token."));
        }
        if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(AppError::validation(
                "A token is a single word; the pasted text has spaces or line breaks in it.",
            ));
        }
        Ok(Token(text.to_string()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Where account tokens are kept. A trait so tests can use memory instead of
/// the user's real Keychain; `Send + Sync` lets one store be shared by threads.
pub trait Keychain: Send + Sync {
    /// Whether the item exists, found without reading the token (no prompt).
    fn contains(&self, kind: ForgeKind) -> AppResult<bool>;
    fn get(&self, kind: ForgeKind) -> AppResult<Option<Token>>;
    /// Create the item, or replace the token in it.
    fn set(&self, kind: ForgeKind, token: &Token) -> AppResult<()>;
    /// Delete the item; an item that is not there is not an error.
    fn delete(&self, kind: ForgeKind) -> AppResult<()>;
}

/// The login keychain of the user running Brainiac.
pub struct MacKeychain;

impl Keychain for MacKeychain {
    fn contains(&self, kind: ForgeKind) -> AppResult<bool> {
        credentials::contains(kind.keychain_account())
    }

    fn get(&self, kind: ForgeKind) -> AppResult<Option<Token>> {
        let Some(bytes) = credentials::read(kind.keychain_account())? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes).map_err(|_| {
            AppError::validation(format!(
                "The Keychain item {SERVICE}/{} does not hold a token.",
                kind.keychain_account()
            ))
        })?;
        Token::new(&text).map(Some)
    }

    fn set(&self, kind: ForgeKind, token: &Token) -> AppResult<()> {
        credentials::write(kind.keychain_account(), token.expose().as_bytes())
    }

    fn delete(&self, kind: ForgeKind) -> AppResult<()> {
        credentials::remove(kind.keychain_account())
    }
}

/// Tokens in memory, for tests.
#[derive(Default)]
pub struct MemoryKeychain {
    // `Mutex` because the trait's methods take `&self` and may run on any thread.
    items: Mutex<HashMap<ForgeKind, Token>>,
}

impl Keychain for MemoryKeychain {
    fn contains(&self, kind: ForgeKind) -> AppResult<bool> {
        Ok(self
            .items
            .lock()
            .expect("keychain lock")
            .contains_key(&kind))
    }

    fn get(&self, kind: ForgeKind) -> AppResult<Option<Token>> {
        Ok(self
            .items
            .lock()
            .expect("keychain lock")
            .get(&kind)
            .cloned())
    }

    fn set(&self, kind: ForgeKind, token: &Token) -> AppResult<()> {
        self.items
            .lock()
            .expect("keychain lock")
            .insert(kind, token.clone());
        Ok(())
    }

    fn delete(&self, kind: ForgeKind) -> AppResult<()> {
        self.items.lock().expect("keychain lock").remove(&kind);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_trimmed_and_never_printed() {
        let token = Token::new("  ghp_secret \n").unwrap();
        assert_eq!(token.expose(), "ghp_secret");
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
        assert!(Token::new("   ").is_err());
        assert!(Token::new("two words").is_err());
    }
}
