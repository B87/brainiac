//! Files in the vault: path validation, folder listings that respect case,
//! and the atomic write a save ends with (docs/architecture.md, Save contract).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::models::{AppError, AppResult};

/// Check a vault-relative path from the frontend or a link: forward slashes,
/// no empty, `.`, or `..` components, no hidden components (Brainiac never
/// works inside `.git` or `.obsidian`), and nothing absolute.
pub fn validate_relative(path: &str) -> AppResult<()> {
    let bad =
        |why: &str| AppError::validation(format!("“{path}” is not a valid vault path: {why}."));
    if path.is_empty() {
        return Err(bad("it is empty"));
    }
    if path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(bad("it must be relative to the vault"));
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(bad("it has an empty or relative component"));
        }
        if part.starts_with('.') {
            return Err(bad("hidden files and folders are not notes"));
        }
        if part.len() > 255 {
            return Err(bad("a name is too long"));
        }
    }
    Ok(())
}

/// `root/relative`, refusing a path that leaves the vault through a symbolic
/// link (docs/architecture.md, Security). Components that do not exist yet are fine.
pub fn resolve(root: &Path, relative: &str) -> AppResult<PathBuf> {
    validate_relative(relative)?;
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(AppError::new(
                    crate::models::ErrorCode::PermissionDenied,
                    format!(
                    "“{relative}” goes through a symbolic link; Brainiac stays inside the vault."
                ),
                ))
            }
            _ => {}
        }
    }
    Ok(current)
}

/// `relative` spelled as the files on disk are: each existing component takes
/// the name its folder lists, which may differ in case from what was asked
/// for (the volume ignores case, so `projects/x.md` lands in `Projects/`).
/// Notes are recorded under that spelling, so a scan finds them again.
pub fn on_disk(root: &Path, relative: &str) -> String {
    let mut dir = root.to_path_buf();
    let mut out: Vec<String> = Vec::new();
    for part in relative.split('/') {
        let listed = fs::read_dir(&dir).ok().and_then(|rd| {
            let names: Vec<String> = rd
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                .collect();
            names
                .iter()
                .find(|n| *n == part)
                .or_else(|| {
                    names
                        .iter()
                        .find(|n| n.to_lowercase() == part.to_lowercase())
                })
                .cloned()
        });
        let name = listed.unwrap_or_else(|| part.to_string());
        dir.push(&name);
        out.push(name);
    }
    out.join("/")
}

/// A file name for a note titled `title`: characters macOS or other tools
/// reject become `-`, leading dots are dropped, and it is shortened.
pub fn file_name_for(title: &str) -> String {
    let cleaned: String = title
        .trim()
        .chars()
        .map(|c| match c {
            '/' | ':' | '\\' | '\0' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim();
    let short: String = cleaned.chars().take(120).collect();
    let short = short.trim();
    if short.is_empty() {
        "Untitled".to_string()
    } else {
        short.to_string()
    }
}

/// Whether a file stem is `name`, or `name` with the number Brainiac adds
/// to tell notes with the same title apart (`Untitled 2`).
pub fn name_matches(stem: &str, name: &str) -> bool {
    stem == name
        || stem
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n >= 2)
}

/// Modification time in nanoseconds since the Unix epoch.
pub fn mtime_ns(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i64)
}

/// RFC 3339 for a modification time in nanoseconds.
pub fn mtime_rfc3339(ns: i64) -> String {
    chrono::DateTime::from_timestamp(
        ns.div_euclid(1_000_000_000),
        ns.rem_euclid(1_000_000_000) as u32,
    )
    .unwrap_or_default()
    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Folder listings read once per reconciliation. Existence is checked against
/// the names a folder lists, not with `stat`: the volume ignores case, so
/// after `Note.md` is renamed `note.md`, `stat` still finds the old name
/// (Decisions, 2 Oct 2026).
#[derive(Default)]
pub struct Listings {
    folders: HashMap<PathBuf, HashSet<OsString>>,
}

impl Listings {
    pub fn exists(&mut self, root: &Path, relative: &str) -> bool {
        let mut dir = root.to_path_buf();
        for part in relative.split('/') {
            let names = self.folders.entry(dir.clone()).or_insert_with(|| {
                fs::read_dir(&dir)
                    .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
                    .unwrap_or_default()
            });
            if !names.contains(std::ffi::OsStr::new(part)) {
                return false;
            }
            dir.push(part);
        }
        true
    }
}

/// A Markdown file found by a walk.
#[derive(Debug, Clone)]
pub struct Found {
    pub relative_path: String,
    pub size: u64,
    pub mtime: i64,
}

/// Every `.md` file under `root/sub`, skipping hidden files and folders and
/// not following symbolic links. Names that are not UTF-8 are skipped.
/// Folders below `sub` that cannot be read are listed in `unreadable`: their
/// notes are not there to see, which is not the same as deleted.
pub fn walk(
    root: &Path,
    sub: &str,
    out: &mut Vec<Found>,
    unreadable: &mut Vec<String>,
) -> std::io::Result<()> {
    let dir = if sub.is_empty() {
        root.to_path_buf()
    } else {
        root.join(sub)
    };
    for entry in fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let rel = if sub.is_empty() {
            name.clone()
        } else {
            format!("{sub}/{name}")
        };
        // `file_type` does not follow symbolic links.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if let Err(e) = walk(root, &rel, out, unreadable) {
                // A folder that vanished mid-walk is gone; one that cannot be read is not.
                if e.kind() != std::io::ErrorKind::NotFound {
                    unreadable.push(rel);
                }
            }
        } else if kind.is_file() && crate::index::is_note_path(&name) {
            if let Ok(meta) = entry.metadata() {
                out.push(Found {
                    relative_path: rel,
                    size: meta.len(),
                    mtime: mtime_ns(&meta),
                });
            }
        }
    }
    Ok(())
}

/// Replace `dest` with `bytes` atomically: write a hidden temporary sibling,
/// flush it, call `still_expected` to re-check the destination just before
/// replacing it, rename it into place, and sync the folder. The temporary
/// file keeps the destination's permissions. A hidden name keeps the vault
/// watcher and other tools from treating it as a note.
pub fn write_atomic(
    dest: &Path,
    bytes: &[u8],
    still_expected: impl FnOnce() -> AppResult<()>,
) -> AppResult<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| AppError::io("The note has no folder."))?;
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{name}.brainiac-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(meta) = fs::metadata(dest) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        drop(file);
        still_expected()?;
        fs::rename(&tmp, dest)?;
        // Make the rename itself durable.
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Write a new file, failing if one exists (any case). Returns false when the
/// name is taken.
pub fn create_new(dest: &Path, bytes: &[u8]) -> AppResult<bool> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Move a file, creating the destination's folders. Fails when the destination exists.
pub fn move_file(from: &Path, to: &Path) -> AppResult<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::symlink_metadata(to).is_ok() && !same_file(from, to) {
        return Err(AppError::new(
            crate::models::ErrorCode::Conflict,
            "A file with that name already exists.",
        ));
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Across volumes (the trash may live on another disk than the vault).
        Err(e) if e.raw_os_error() == Some(18) => {
            fs::copy(from, to)?;
            fs::remove_file(from)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

/// Whether two paths name the same file, as after a case-only rename.
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numbered_name_still_matches_its_title() {
        assert!(name_matches("Untitled", "Untitled"));
        assert!(name_matches("Untitled 2", "Untitled"));
        assert!(name_matches("Plan 12", "Plan"));
        for (stem, name) in [
            ("Untitled 1", "Untitled"),
            ("Untitled 2b", "Untitled"),
            ("Untitled2", "Untitled"),
            ("2026-10-02", "Standup"),
            ("plan", "Plan"),
        ] {
            assert!(!name_matches(stem, name), "{stem} / {name}");
        }
    }

    #[test]
    fn vault_paths_are_validated() {
        for ok in ["a.md", "Folder/Sub/Note.md", "Café/ñ.md"] {
            assert!(validate_relative(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/abs.md",
            "a/../b.md",
            "./a.md",
            "a//b.md",
            ".git/config",
            "a/.hidden.md",
            "a\\b.md",
        ] {
            assert!(validate_relative(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_are_recorded_as_spelled_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Projects")).unwrap();
        fs::write(tmp.path().join("Projects/Roadmap.md"), "x").unwrap();
        assert_eq!(
            on_disk(tmp.path(), "projects/Roadmap.md"),
            "Projects/Roadmap.md"
        );
        assert_eq!(on_disk(tmp.path(), "projects/New.md"), "Projects/New.md");
        assert_eq!(on_disk(tmp.path(), "Other/New.md"), "Other/New.md");
    }

    #[test]
    fn titles_become_safe_file_names() {
        assert_eq!(file_name_for("Plan: Q4/Q1"), "Plan- Q4-Q1");
        assert_eq!(file_name_for("  ..hidden"), "hidden");
        assert_eq!(file_name_for("   "), "Untitled");
    }

    #[test]
    fn a_symlink_out_of_the_vault_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, vault.join("link")).unwrap();
        assert!(resolve(&vault, "link/secret.md").is_err());
        assert!(resolve(&vault, "real/new.md").is_ok());
    }

    #[test]
    fn an_atomic_write_keeps_permissions_and_leaves_no_temporary_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("n.md");
        fs::write(&dest, "old").unwrap();
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&dest, b"new", || Ok(())).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "new");
        assert_eq!(
            fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let refused = write_atomic(&dest, b"newer", || Err(AppError::validation("changed")));
        assert!(refused.is_err());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "new");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
}
