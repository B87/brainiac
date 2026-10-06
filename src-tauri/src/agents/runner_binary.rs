//! The Linux `brainiac-runner` binary, built on this Mac in Docker and
//! cached by a digest of the sources (docs/architecture.md, Remote hosts).
//! The remote host does not compile it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use sha2::{Digest, Sha256};

use crate::models::{AppError, AppResult};

/// `linux/amd64` or `linux/arm64` for a host's `uname -m`.
pub fn platform(uname: &str) -> AppResult<&'static str> {
    match uname.trim() {
        "x86_64" => Ok("linux/amd64"),
        "aarch64" | "arm64" => Ok("linux/arm64"),
        _ => Err(AppError::dependency(
            "The host's processor is not one Brainiac builds a run controller for.",
        )
        .with_details(uname.trim().to_string())),
    }
}

/// The cached binary for this checkout and platform, building it when missing.
pub async fn ensure(cache_dir: &Path, uname: &str) -> AppResult<PathBuf> {
    let platform = platform(uname)?;
    let root = source_root()?;
    let digest = source_digest(&root)?;
    std::fs::create_dir_all(cache_dir)?;
    let cached = cache_dir.join(format!(
        "brainiac-runner-{}-{}",
        platform.replace('/', "-"),
        &digest[..16]
    ));
    if cached.is_file() {
        return Ok(cached);
    }
    build(&root, platform, &cached).await?;
    Ok(cached)
}

/// SHA-256 of the cached file, hex.
pub fn file_digest(path: &Path) -> AppResult<String> {
    let bytes = std::fs::read(path)?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn source_root() -> AppResult<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| {
        AppError::dependency("Brainiac could not find its own program.").with_details(e.to_string())
    })?;
    let mut dir = exe.as_path();
    for _ in 0..8 {
        let Some(parent) = dir.parent() else { break };
        dir = parent;
        let manifest = dir.join("src-tauri/Cargo.toml");
        let controller = dir.join("src-tauri/src/agents/controller.rs");
        if manifest.is_file() && controller.is_file() {
            return Ok(dir.join("src-tauri"));
        }
    }
    Err(AppError::dependency(
        "The Linux run controller is built from a Brainiac checkout. Deploy from the repository.",
    ))
}

fn source_digest(root: &Path) -> AppResult<String> {
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files)?;
    for name in ["Cargo.toml", "Cargo.lock", "build.rs"] {
        let path = root.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    let toolchain = root.join("../rust-toolchain.toml");
    if toolchain.is_file() {
        files.push(toolchain);
    }
    files.sort();
    let mut hasher = Sha256::new();
    for path in files {
        let bytes = std::fs::read(&path)?;
        hasher.update(path.display().to_string().as_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> AppResult<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

async fn build(root: &Path, platform: &str, dest: &Path) -> AppResult<()> {
    let repo = root
        .parent()
        .ok_or_else(|| AppError::dependency("The checkout is incomplete."))?;
    let script = "cargo build --locked --release --no-default-features --bin brainiac-runner --target-dir /tmp/brainiac-target && cp /tmp/brainiac-target/release/brainiac-runner /out/brainiac-runner";
    // A login shell (`bash -l`) resets PATH and drops the image's cargo.
    let output = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--platform",
            platform,
            "-v",
            &format!("{}:/src", repo.display()),
            "-v",
            &format!(
                "{}:/out",
                dest.parent().unwrap_or(Path::new("/tmp")).display()
            ),
            "-w",
            "/src/src-tauri",
            "rust:1.88",
            "bash",
            "-c",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            AppError::dependency("Docker could not build the run controller.")
                .with_details(e.to_string())
        })?;
    if !output.status.success() {
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return Err(
            AppError::dependency("The Linux run controller did not build.")
                .with_details(log_tail(&log)),
        );
    }
    let built = dest
        .parent()
        .unwrap_or(Path::new("/tmp"))
        .join("brainiac-runner");
    if built != dest {
        std::fs::rename(&built, dest)?;
    }
    if !dest.is_file() {
        return Err(AppError::dependency(
            "The Linux run controller was not written.",
        ));
    }
    Ok(())
}

/// The end of a failed build log. The whole log is too long to keep.
fn log_tail(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let start = lines.len().saturating_sub(40);
    lines[start..].join("\n")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_build_log_keeps_its_ending() {
        let log = (0..50)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = log_tail(&log);
        assert!(tail.contains("line 49"));
        assert!(!tail.contains("line 0"));
    }
}
