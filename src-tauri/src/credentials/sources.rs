//! Checking a source's reference before it is saved or tested, describing it
//! safely, and reading the sources that are not the store.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::command::program_name;
use super::{item_name, SecretBytes, MAX_SECRET_BYTES};
use crate::models::{AppError, AppResult, SecretSource};

const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 4096;

/// Check a source's reference: a variable's name, or a program's absolute
/// path and its arguments. Which sources an owner may use is the domain's
/// rule, not this one's. Returns the source with surrounding spaces of the
/// name and program removed; arguments are kept exactly.
pub fn check_source(source: &SecretSource) -> AppResult<SecretSource> {
    match source {
        SecretSource::Environment { name } => {
            let name = name.trim();
            let mut chars = name.chars();
            let valid = chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid || name.len() > 255 {
                return Err(AppError::validation(
                    "An environment variable's name has letters, digits, and underscores, and does not start with a digit.",
                ));
            }
            Ok(SecretSource::Environment {
                name: name.to_string(),
            })
        }
        SecretSource::Command { program, args } => {
            let program = program.trim();
            if !Path::new(program).is_absolute() {
                return Err(AppError::validation(
                    "Choose the program with its full path; Find… looks it up.",
                ));
            }
            if !std::fs::metadata(program).is_ok_and(|m| m.is_file()) {
                return Err(AppError::not_found(format!(
                    "There is no program at {program}."
                )));
            }
            if args.len() > MAX_ARGS {
                return Err(AppError::validation(format!(
                    "A command has at most {MAX_ARGS} arguments."
                )));
            }
            if args
                .iter()
                .any(|a| a.len() > MAX_ARG_BYTES || a.contains('\0'))
            {
                return Err(AppError::validation(format!(
                    "Each argument is at most {MAX_ARG_BYTES} bytes, without a NUL character."
                )));
            }
            Ok(SecretSource::Command {
                program: program.to_string(),
                args: args.clone(),
            })
        }
        other => Ok(other.clone()),
    }
}

/// Where a secret comes from, in words safe for an error or a log: the
/// Keychain item, the variable's name, or the program's file name, never a
/// command's arguments.
pub fn describe(source: &SecretSource, owner: &str) -> String {
    match source {
        SecretSource::Store => format!("the Keychain item {}", item_name(owner)),
        SecretSource::Ask => "the password typed for this run".to_string(),
        SecretSource::None => "no secret".to_string(),
        SecretSource::Environment { name } => format!("the environment variable {name}"),
        SecretSource::Command { program, .. } => format!("the command {}", program_name(program)),
    }
}

/// An environment variable of Brainiac's own process, as it was at launch.
pub fn environment(name: &str) -> AppResult<SecretBytes> {
    let value = std::env::var_os(name).ok_or_else(|| {
        AppError::not_found(format!(
            "Brainiac's environment has no variable {name}. An app opened from Finder or the Dock does not get a shell's variables; use a command instead."
        ))
    })?;
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return Err(AppError::validation(format!(
            "The environment variable {name} is empty."
        )));
    }
    if bytes.len() > MAX_SECRET_BYTES {
        return Err(AppError::validation(format!(
            "The environment variable {name} holds more than {} KiB.",
            MAX_SECRET_BYTES / 1024
        )));
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(AppError::validation(format!(
            "The environment variable {name} is not valid UTF-8 text."
        )));
    }
    Ok(SecretBytes::new(bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_checked_and_described_without_arguments() {
        let env = |name: &str| SecretSource::Environment { name: name.into() };
        assert_eq!(
            check_source(&env(" GITHUB_TOKEN ")).unwrap(),
            env("GITHUB_TOKEN")
        );
        assert!(check_source(&env("1TOKEN")).is_err());
        assert!(check_source(&env("TO KEN")).is_err());
        assert!(check_source(&env("")).is_err());

        let command = SecretSource::Command {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), " spaced ".into()],
        };
        assert_eq!(check_source(&command).unwrap(), command);
        let relative = SecretSource::Command {
            program: "sh".into(),
            args: vec![],
        };
        assert!(check_source(&relative).is_err());
        let nul = SecretSource::Command {
            program: "/bin/sh".into(),
            args: vec!["a\0b".into()],
        };
        assert!(check_source(&nul).is_err());

        let op = SecretSource::Command {
            program: "/opt/homebrew/bin/op".into(),
            args: vec!["read".into(), "op://Private/hunter2".into()],
        };
        assert_eq!(describe(&op, "github"), "the command op");
        assert!(!format!("{op:?}").contains("hunter2"));
        assert_eq!(
            describe(&SecretSource::Store, "db:1"),
            "the Keychain item brainiac/db:1"
        );
    }

    #[test]
    fn environment_values_are_read_and_their_absence_is_explained() {
        // A name no other test or process uses.
        let name = "BRAINIAC_TEST_SECRET_SOURCE_VALUE";
        assert_eq!(
            environment(name).unwrap_err().code,
            crate::models::ErrorCode::NotFound
        );
        std::env::set_var(name, " s3cret ");
        assert_eq!(environment(name).unwrap().expose(), b" s3cret ".as_slice());
        std::env::set_var(name, "");
        assert!(environment(name).unwrap_err().message.contains("empty"));
        std::env::remove_var(name);
    }
}
