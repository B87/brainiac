//! Where secrets come from (SPEC.md, Secrets; docs/architecture.md,
//! Credentials): account tokens and database passwords, each read from the
//! source its owner saved.
//!
//! - **The store** is the one place Brainiac writes secrets: generic passwords
//!   in the login keychain with service `brainiac` and an account naming the
//!   owner (`github`, `bitbucket`, `db:<connection id>`). `SecretStore` is its
//!   interface; `MacStore` the Keychain, `MemoryStore` the tests'.
//! - **Sources** are read and never written: the store, a password typed for
//!   this run (Ask), an environment variable, or a command's output.
//! - **`CredentialService`** resolves a binding (an owner, its source, and the
//!   revision of both) to a lease, caches it for the run, shares concurrent
//!   reads, and forgets exactly the lease a server refused.
//!
//! Every value is `SecretBytes` until a domain service turns it into the
//! `Token` or `Secret` it sends, with that domain's rules.

mod command;
#[cfg(target_os = "macos")]
mod macos;
mod service;
mod sources;
mod store;

use std::fmt;

pub use command::{find_program, CommandRunner};
#[cfg(target_os = "macos")]
pub use macos::MacStore;
pub use service::{Binding, CredentialService, Lease, LeaseHandle, OwnerGate, Probe, Resolution};
pub use sources::{check_source, describe};
pub use store::{MemoryStore, SecretStore};

use crate::models::{AppError, AppResult};

/// The Keychain service name of every item.
pub const SERVICE: &str = "brainiac";

/// The most bytes any source may return.
pub const MAX_SECRET_BYTES: usize = 64 * 1024;

/// The store item of an owner, as macOS's Keychain Access shows it.
pub fn item_name(key: &str) -> String {
    format!("{SERVICE}/{key}")
}

/// Secret bytes as a source returned them. No `Display`, a `Debug` that
/// prints nothing, no serialization, and zeroed when dropped (as far as Rust
/// lets a program promise that), so a value cannot reach a log by accident.
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        SecretBytes(bytes)
    }

    pub fn from_text(text: &str) -> Self {
        SecretBytes(text.as_bytes().to_vec())
    }

    /// The bytes, only to decode them, write them to the store, or authenticate.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The bytes as text, for the domain wrappers below.
    fn text(&self, what: &str, source: &str) -> AppResult<&str> {
        std::str::from_utf8(&self.0)
            .map_err(|_| AppError::validation(format!("The {what} from {source} is not text.")))
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        // Overwrite before the allocator reuses the memory.
        self.0.iter_mut().for_each(|b| *b = 0);
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes(<redacted>)")
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

    /// A password a source returned, with the same rules as a typed one.
    pub fn from_bytes(bytes: &SecretBytes, source: &str) -> AppResult<Self> {
        let text = bytes.text("password", source)?;
        if text.is_empty() {
            return Err(AppError::validation(format!(
                "The password from {source} is empty."
            )));
        }
        Secret::new(text)
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

/// An account token. Like `Secret`, it cannot be printed; `expose` is called
/// only where it goes into a request header or the Keychain.
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

    /// A token a source returned, with the same rules as a pasted one.
    pub fn from_bytes(bytes: &SecretBytes, source: &str) -> AppResult<Self> {
        let text = bytes.text("token", source)?;
        Token::new(text).map_err(|_| {
            AppError::validation(format!(
                "What {source} returned is not a token: a token is one word, without spaces or line breaks."
            ))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_keep_spaces_and_are_never_printed() {
        let secret = Secret::new(" pass word ").unwrap();
        assert_eq!(secret.expose(), " pass word ");
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert!(Secret::new("").is_err());
        let bytes = SecretBytes::from_text(" pass word ");
        assert_eq!(format!("{bytes:?}"), "SecretBytes(<redacted>)");
        assert_eq!(
            Secret::from_bytes(&bytes, "the Keychain").unwrap().expose(),
            " pass word "
        );
        let invalid = SecretBytes::new(vec![0xff, 0xfe]);
        let err = Secret::from_bytes(&invalid, "the Keychain").unwrap_err();
        assert!(err.message.contains("not text"), "{err:?}");
    }

    #[test]
    fn tokens_are_trimmed_and_never_printed() {
        let token = Token::new("  ghp_secret \n").unwrap();
        assert_eq!(token.expose(), "ghp_secret");
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
        assert!(Token::new("   ").is_err());
        assert!(Token::new("two words").is_err());
        let err = Token::from_bytes(&SecretBytes::from_text("two secret words"), "gh").unwrap_err();
        assert!(!err.message.contains("secret"), "{err:?}");
    }
}
