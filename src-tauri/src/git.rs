//! Git inspection through the system `git` binary.
//!
//! Every call builds an argument array (never a shell string), runs with a
//! timeout, disables pagers/hooks/external helpers, and parses
//! machine-readable output into the DTOs from `models.rs`. Nothing here
//! writes to a repository.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::models::{
    AppError, AppResult, ChangeCounts, ChangeEntry, ChangeGroup, ChangeKind, CommitSummary,
    DiffContent, DiffLimits, DiffLine, DiffLineKind, DiffSelector, HeadKind, HeadState, Hunk,
    NonTextKind, StatusSnapshot, UpstreamState,
};

/// Minimum supported Git version (SPEC §3).
pub const MIN_GIT_VERSION: (u32, u32) = (2, 30);

/// Hard cap on bytes read from any subprocess; larger output is cut and the
/// child is killed. Diff display limits are applied separately and are lower.
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Field and record separators used in custom `git log` formats.
const FIELD_SEP: char = '\u{1f}';

#[derive(Debug, Clone)]
pub struct GitService {
    binary: PathBuf,
    version: String,
    timeout: Duration,
}

/// What `git rev-parse` reports for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRepository {
    /// Working-tree root, canonicalized.
    pub root: PathBuf,
    /// Per-worktree Git directory (`.git` or `.git/worktrees/<name>`).
    pub git_dir: PathBuf,
    /// Shared Git directory (same as `git_dir` for a normal checkout).
    pub common_git_dir: PathBuf,
}

impl ResolvedRepository {
    pub fn is_linked_worktree(&self) -> bool {
        self.git_dir != self.common_git_dir
    }
}

impl GitService {
    /// Locate `git` on PATH and verify its version.
    pub async fn detect() -> AppResult<Self> {
        Self::detect_at(Path::new("git")).await
    }

    pub async fn detect_at(binary: &Path) -> AppResult<Self> {
        let probe = GitService {
            binary: binary.to_path_buf(),
            version: String::new(),
            timeout: Duration::from_secs(5),
        };
        let out = probe.run_raw(None, &["--version"]).await.map_err(|e| {
            AppError::dependency("Git was not found on PATH. Install the Xcode Command Line Tools or Git, then restart Brainiac.")
                .with_details(e.to_string())
        })?;
        let text = String::from_utf8_lossy(&out).trim().to_string();
        let version = parse_version(&text).ok_or_else(|| {
            AppError::dependency("Git reported an unexpected version string.")
                .with_details(text.clone())
        })?;
        if (version.0, version.1) < MIN_GIT_VERSION {
            return Err(AppError::dependency(format!(
                "Git {}.{} is too old; Brainiac needs {}.{} or newer.",
                version.0, version.1, MIN_GIT_VERSION.0, MIN_GIT_VERSION.1
            ))
            .with_details(text));
        }
        Ok(GitService {
            version: text,
            ..probe
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    // -----------------------------------------------------------------------
    // Subprocess plumbing
    // -----------------------------------------------------------------------

    fn command(&self, cwd: Option<&Path>) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.args([
            "--no-pager",
            "-c",
            "core.quotePath=false",
            "-c",
            "color.ui=never",
        ]);
        cmd.env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd
    }

    /// Run Git and return stdout. Non-zero exit becomes an error carrying stderr.
    async fn run_raw(&self, cwd: Option<&Path>, args: &[&str]) -> AppResult<Vec<u8>> {
        let (stdout, _truncated) = self.run_bounded(cwd, args, MAX_OUTPUT_BYTES).await?;
        Ok(stdout)
    }

    /// Run Git, reading at most `max_bytes` of stdout. Returns `(stdout, truncated)`.
    async fn run_bounded(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        max_bytes: usize,
    ) -> AppResult<(Vec<u8>, bool)> {
        let mut cmd = self.command(cwd);
        cmd.args(args);
        let mut child = cmd.spawn().map_err(|e| {
            AppError::dependency("Could not start Git.")
                .with_details(format!("{}: {e}", self.binary.display()))
        })?;
        let mut stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");

        let read_all = async {
            let mut out = Vec::new();
            let mut buf = vec![0u8; 64 * 1024];
            let mut truncated = false;
            loop {
                let n = stdout.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                if out.len() + n > max_bytes {
                    out.extend_from_slice(&buf[..max_bytes.saturating_sub(out.len())]);
                    truncated = true;
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            }
            Ok::<_, std::io::Error>((out, truncated))
        };
        let read_err = async {
            let mut err = Vec::new();
            let _ = stderr.read_to_end(&mut err).await;
            err
        };

        let result = tokio::time::timeout(self.timeout, async {
            let (read, err) = tokio::join!(read_all, read_err);
            let (out, truncated) = read?;
            if truncated {
                let _ = child.kill().await;
            }
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((out, truncated, err, status))
        })
        .await;

        match result {
            Err(_) => {
                let _ = child.kill().await;
                Err(AppError::timeout(format!(
                    "Git did not answer within {} seconds.",
                    self.timeout.as_secs()
                ))
                .with_details(format!("git {}", args.join(" "))))
            }
            Ok(Err(e)) => {
                Err(AppError::io("Reading Git output failed.").with_details(e.to_string()))
            }
            Ok(Ok((out, truncated, err, status))) => {
                if status.success() || truncated {
                    Ok((out, truncated))
                } else {
                    let stderr = String::from_utf8_lossy(&err).trim().to_string();
                    Err(classify_git_error(args, &stderr))
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Resolve the repository containing `path`. Fails for non-Git paths.
    pub async fn resolve(&self, path: &Path) -> AppResult<ResolvedRepository> {
        let meta = tokio::fs::metadata(path).await.map_err(|e| {
            AppError::not_found(format!(
                "The folder {} does not exist or cannot be read.",
                path.display()
            ))
            .with_details(e.to_string())
        })?;
        if !meta.is_dir() {
            return Err(AppError::validation(format!(
                "{} is not a folder.",
                path.display()
            )));
        }
        let out = self
            .run_raw(
                Some(path),
                &[
                    "rev-parse",
                    "--show-toplevel",
                    "--absolute-git-dir",
                    "--git-common-dir",
                ],
            )
            .await
            .map_err(|e| match e.code {
                crate::models::ErrorCode::Validation | crate::models::ErrorCode::NotFound => {
                    AppError::validation(format!(
                        "{} is not inside a Git repository.",
                        path.display()
                    ))
                    .with_details(e.details.unwrap_or_default())
                }
                _ => e,
            })?;
        let text = String::from_utf8_lossy(&out);
        let mut lines = text.lines();
        let root = lines.next().ok_or_else(|| {
            AppError::validation("Git did not report a working-tree root (bare repository?).")
        })?;
        let git_dir = lines.next().unwrap_or_default();
        let common = lines.next().unwrap_or_default();
        let root = canonicalize(Path::new(root)).await?;
        let git_dir = canonicalize(&absolute_in(&root, git_dir)).await?;
        let common_git_dir = canonicalize(&absolute_in(&root, common)).await?;
        Ok(ResolvedRepository {
            root,
            git_dir,
            common_git_dir,
        })
    }

    // -----------------------------------------------------------------------
    // Status
    // -----------------------------------------------------------------------

    pub async fn status(&self, root: &Path) -> AppResult<StatusSnapshot> {
        let out = self
            .run_raw(
                Some(root),
                &[
                    "status",
                    "--porcelain=v2",
                    "--branch",
                    "--untracked-files=normal",
                    "-z",
                ],
            )
            .await?;
        let mut snapshot = parse_status_v2(&out)?;
        snapshot.last_commit_at = match snapshot.head.commit_id.as_deref() {
            Some(id) => self
                .run_raw(Some(root), &["log", "-1", "--format=%cI", id, "--"])
                .await
                .ok()
                .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                .filter(|s| !s.is_empty()),
            None => None,
        };
        Ok(snapshot)
    }

    // -----------------------------------------------------------------------
    // History
    // -----------------------------------------------------------------------

    /// Resolve a ref (or commit id) to a full commit id. `None` for an unborn HEAD.
    pub async fn resolve_commit(&self, root: &Path, ref_name: &str) -> AppResult<Option<String>> {
        validate_revision(ref_name)?;
        let spec = format!("{ref_name}^{{commit}}");
        match self
            .run_raw(
                Some(root),
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &spec,
                ],
            )
            .await
        {
            Ok(out) => Ok(Some(String::from_utf8_lossy(&out).trim().to_string())),
            // Infrastructure problems propagate unchanged.
            Err(e)
                if matches!(
                    e.code,
                    crate::models::ErrorCode::Timeout
                        | crate::models::ErrorCode::DependencyUnavailable
                        | crate::models::ErrorCode::PermissionDenied
                ) =>
            {
                Err(e)
            }
            // Any other failure means the revision does not resolve; `--quiet`
            // suppresses Git's message, so an unborn HEAD arrives here with empty stderr.
            Err(_) if ref_name == "HEAD" => Ok(None),
            Err(e) => Err(
                AppError::not_found(format!("Ref {ref_name} was not found."))
                    .with_details(e.details.unwrap_or_default()),
            ),
        }
    }

    /// Commits reachable from `anchor`, newest first, skipping `offset`.
    /// Returns up to `limit + 1` so the caller can tell whether another page exists.
    pub async fn log(
        &self,
        root: &Path,
        anchor: &str,
        offset: u32,
        limit: u32,
        filter: Option<&str>,
    ) -> AppResult<Vec<CommitSummary>> {
        validate_revision(anchor)?;
        let format = format!(
            "--format=%H{s}%h{s}%P{s}%an{s}%ae{s}%aI{s}%cI{s}%D{s}%s",
            s = FIELD_SEP
        );
        let skip = format!("--skip={offset}");
        let count = format!("--max-count={}", limit + 1);
        let mut args: Vec<&str> = vec!["log", "-z", &format, &skip, &count];
        let grep;
        if let Some(f) = filter.map(str::trim).filter(|f| !f.is_empty()) {
            grep = format!("--grep={f}");
            args.extend(["--regexp-ignore-case", "--fixed-strings", &grep]);
        }
        args.extend(["--end-of-options", anchor, "--"]);
        let out = self.run_raw(Some(root), &args).await?;
        Ok(parse_log(&out))
    }

    /// Fallback for hash filters: a single commit whose id starts with `prefix`.
    pub async fn log_by_hash_prefix(
        &self,
        root: &Path,
        anchor: &str,
        prefix: &str,
    ) -> AppResult<Vec<CommitSummary>> {
        if !is_hex(prefix) || prefix.len() < 4 {
            return Ok(Vec::new());
        }
        let Some(id) = self.resolve_commit(root, prefix).await.ok().flatten() else {
            return Ok(Vec::new());
        };
        validate_revision(anchor)?;
        // Only accept it when it is reachable from the anchor.
        let reachable = self
            .run_raw(Some(root), &["merge-base", "--is-ancestor", &id, anchor])
            .await
            .is_ok();
        if !reachable {
            return Ok(Vec::new());
        }
        self.log(root, &id, 0, 1, None).await.map(|mut v| {
            v.truncate(1);
            v
        })
    }

    // -----------------------------------------------------------------------
    // Diffs
    // -----------------------------------------------------------------------

    pub async fn diff(
        &self,
        root: &Path,
        selector: &DiffSelector,
        limits: &DiffLimits,
    ) -> AppResult<DiffContent> {
        let path = selector.path();
        validate_repo_path(path)?;
        let max_bytes = limits.max_bytes as usize;
        let common = [
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
            "--find-renames",
        ];
        let (raw, truncated) = match selector {
            DiffSelector::WorktreeVsIndex { .. } => {
                let mut args = vec!["diff"];
                args.extend(common);
                args.extend(["--", path]);
                self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                    .await?
            }
            DiffSelector::IndexVsHead { .. } => {
                let mut args = vec!["diff", "--cached"];
                args.extend(common);
                args.extend(["--", path]);
                self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                    .await?
            }
            DiffSelector::UntrackedPreview { .. } => {
                return self.untracked_preview(root, path, limits).await;
            }
            DiffSelector::Commit {
                commit_id,
                parent_index,
                ..
            } => {
                validate_revision(commit_id)?;
                if *parent_index == 0 {
                    // `show` compares with the first parent, or the empty tree for a root commit.
                    let mut args = vec!["show", "--format=", "--first-parent"];
                    args.extend(common);
                    args.extend(["--end-of-options", commit_id, "--", path]);
                    self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                        .await?
                } else {
                    let parent = format!("{commit_id}^{parent_index}");
                    let mut args = vec!["diff"];
                    args.extend(common);
                    args.extend(["--end-of-options", &parent, commit_id, "--", path]);
                    self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                        .await?
                }
            }
        };
        Ok(parse_unified_diff(&raw, truncated, limits))
    }

    async fn untracked_preview(
        &self,
        root: &Path,
        path: &str,
        limits: &DiffLimits,
    ) -> AppResult<DiffContent> {
        let full = root.join(path);
        let meta = tokio::fs::symlink_metadata(&full).await?;
        if meta.file_type().is_symlink() {
            let target = tokio::fs::read_link(&full).await?;
            return Ok(DiffContent::NonText {
                reason: NonTextKind::Symlink,
                summary: format!("Symbolic link to {}", target.display()),
                byte_size: None,
            });
        }
        if meta.is_dir() {
            return Ok(DiffContent::NonText {
                reason: NonTextKind::Submodule,
                summary: "Untracked directory".into(),
                byte_size: None,
            });
        }
        let size = meta.len();
        let mut file = tokio::fs::File::open(&full).await?;
        let cap = (limits.max_bytes as usize).min(MAX_OUTPUT_BYTES);
        let mut buf = Vec::with_capacity(size.min(cap as u64) as usize);
        let mut chunk = vec![0u8; 64 * 1024];
        let mut truncated = false;
        loop {
            let n = file.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            if buf.len() + n > cap {
                buf.extend_from_slice(&chunk[..cap - buf.len()]);
                truncated = true;
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        if looks_binary(&buf) {
            return Ok(DiffContent::NonText {
                reason: NonTextKind::Binary,
                summary: format!("Binary file, {} bytes", size),
                byte_size: Some(size),
            });
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines: Vec<DiffLine> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if lines.len() as u64 >= limits.max_lines {
                truncated = true;
                break;
            }
            lines.push(DiffLine {
                kind: DiffLineKind::Add,
                old_no: None,
                new_no: Some(i as u64 + 1),
                text: line.to_string(),
            });
        }
        let total = text.lines().count() as u64;
        let hunk = Hunk {
            header: "untracked".into(),
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: lines.len() as u64,
            lines,
        };
        Ok(DiffContent::Text {
            old_path: None,
            new_path: Some(path.to_string()),
            hunks: if hunk.lines.is_empty() {
                vec![]
            } else {
                vec![hunk]
            },
            truncated,
            total_lines: Some(total),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers and parsers (pure functions; unit-tested below)
// ---------------------------------------------------------------------------

async fn canonicalize(path: &Path) -> AppResult<PathBuf> {
    tokio::fs::canonicalize(path).await.map_err(|e| {
        AppError::io(format!("Could not resolve {}.", path.display())).with_details(e.to_string())
    })
}

fn absolute_in(base: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    // "git version 2.54.0 (Apple Git-157)"
    let token = text.split_whitespace().nth(2)?;
    let mut parts = token
        .split('.')
        .map(|p| p.trim_end_matches(|c: char| !c.is_ascii_digit()));
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

fn classify_git_error(args: &[&str], stderr: &str) -> AppError {
    let lower = stderr.to_ascii_lowercase();
    let cmd = format!("git {}", args.join(" "));
    if lower.contains("not a git repository") {
        AppError::validation("This folder is not a Git repository.")
            .with_details(format!("{cmd}\n{stderr}"))
    } else if lower.contains("unknown revision")
        || lower.contains("bad revision")
        || lower.contains("needed a single revision")
        || lower.contains("ambiguous argument")
        || lower.contains("does not have any commits")
    {
        AppError::not_found("The requested revision does not exist.")
            .with_details(format!("{cmd}\n{stderr}"))
    } else if lower.contains("permission denied") {
        AppError::new(
            crate::models::ErrorCode::PermissionDenied,
            "Git could not read the repository.",
        )
        .with_details(format!("{cmd}\n{stderr}"))
    } else {
        AppError::io("Git returned an error.").with_details(format!("{cmd}\n{stderr}"))
    }
}

/// Revisions must never look like options and must not contain path separators
/// that would let a caller escape the repository.
pub fn validate_revision(rev: &str) -> AppResult<()> {
    if rev.is_empty() || rev.starts_with('-') || rev.contains(['\0', '\n']) || rev.contains("..") {
        return Err(AppError::validation(format!("Invalid revision: {rev:?}")));
    }
    Ok(())
}

/// Repository-relative paths must stay inside the working tree.
pub fn validate_repo_path(path: &str) -> AppResult<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\0') {
        return Err(AppError::validation(format!("Invalid path: {path:?}")));
    }
    if Path::new(path).components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        )
    }) {
        return Err(AppError::validation(format!(
            "Path escapes the repository: {path:?}"
        )));
    }
    Ok(())
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

fn kind_from_char(c: char) -> ChangeKind {
    match c {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::TypeChanged,
        'U' => ChangeKind::Unmerged,
        _ => ChangeKind::Modified,
    }
}

/// Parse `git status --porcelain=v2 --branch -z`.
pub fn parse_status_v2(bytes: &[u8]) -> AppResult<StatusSnapshot> {
    let mut head = HeadState {
        kind: HeadKind::Unborn,
        branch: None,
        commit_id: None,
    };
    let mut upstream_ref: Option<String> = None;
    let mut ahead_behind: Option<(u64, u64)> = None;
    let mut entries: Vec<ChangeEntry> = Vec::new();
    let mut counts = ChangeCounts::default();
    let mut unique: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut records = bytes.split(|&b| b == 0).peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(record).into_owned();
        if let Some(rest) = line.strip_prefix("# ") {
            let mut parts = rest.splitn(2, ' ');
            let key = parts.next().unwrap_or_default();
            let value = parts.next().unwrap_or_default().trim();
            match key {
                "branch.oid" => {
                    if value != "(initial)" {
                        head.commit_id = Some(value.to_string());
                        if head.kind == HeadKind::Unborn {
                            head.kind = HeadKind::Detached;
                        }
                    }
                }
                "branch.head" => {
                    if value == "(detached)" {
                        head.kind = HeadKind::Detached;
                    } else {
                        head.branch = Some(value.to_string());
                        head.kind = if head.commit_id.is_some() {
                            HeadKind::Branch
                        } else {
                            HeadKind::Unborn
                        };
                    }
                }
                "branch.upstream" => upstream_ref = Some(value.to_string()),
                "branch.ab" => {
                    let mut ab = value.split_whitespace();
                    let a = ab
                        .next()
                        .and_then(|s| s.trim_start_matches('+').parse().ok())
                        .unwrap_or(0);
                    let b = ab
                        .next()
                        .and_then(|s| s.trim_start_matches('-').parse().ok())
                        .unwrap_or(0);
                    ahead_behind = Some((a, b));
                }
                _ => {}
            }
            continue;
        }

        let mut fields = line.splitn(2, ' ');
        let tag = fields.next().unwrap_or_default();
        let rest = fields.next().unwrap_or_default();
        match tag {
            "1" | "2" => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
                let parts: Vec<&str> = rest.splitn(if tag == "1" { 8 } else { 9 }, ' ').collect();
                let (xy, sub, path) = match tag {
                    "1" if parts.len() == 8 => (parts[0], parts[1], parts[7]),
                    "2" if parts.len() == 9 => (parts[0], parts[1], parts[8]),
                    _ => continue,
                };
                let old_path = if tag == "2" {
                    records
                        .next()
                        .map(|r| String::from_utf8_lossy(r).into_owned())
                } else {
                    None
                };
                let is_submodule = sub.starts_with('S');
                let mut xy_chars = xy.chars();
                let x = xy_chars.next().unwrap_or('.');
                let y = xy_chars.next().unwrap_or('.');
                unique.insert(path.to_string());
                if x != '.' {
                    counts.staged += 1;
                    entries.push(ChangeEntry {
                        group: ChangeGroup::Staged,
                        kind: kind_from_char(x),
                        path: path.to_string(),
                        old_path: old_path.clone(),
                        is_submodule,
                    });
                }
                if y != '.' {
                    counts.unstaged += 1;
                    entries.push(ChangeEntry {
                        group: ChangeGroup::Unstaged,
                        kind: kind_from_char(y),
                        path: path.to_string(),
                        old_path: None,
                        is_submodule,
                    });
                }
            }
            "u" => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let parts: Vec<&str> = rest.splitn(10, ' ').collect();
                if parts.len() == 10 {
                    let path = parts[9];
                    unique.insert(path.to_string());
                    counts.conflicted += 1;
                    entries.push(ChangeEntry {
                        group: ChangeGroup::Conflicted,
                        kind: ChangeKind::Unmerged,
                        path: path.to_string(),
                        old_path: None,
                        is_submodule: parts[1].starts_with('S'),
                    });
                }
            }
            "?" => {
                unique.insert(rest.to_string());
                counts.untracked += 1;
                entries.push(ChangeEntry {
                    group: ChangeGroup::Untracked,
                    kind: ChangeKind::Untracked,
                    path: rest.to_string(),
                    old_path: None,
                    is_submodule: false,
                });
            }
            _ => {}
        }
    }
    counts.unique_paths = unique.len() as u64;
    let upstream = upstream_ref.map(|r| UpstreamState {
        ref_name: r,
        ahead: ahead_behind.map(|ab| ab.0).unwrap_or(0),
        behind: ahead_behind.map(|ab| ab.1).unwrap_or(0),
    });
    Ok(StatusSnapshot {
        observed_at: crate::models::now_rfc3339(),
        head,
        counts,
        upstream,
        last_commit_at: None,
        entries,
    })
}

/// Parse `git log -z --format=<fields separated by 0x1f>`.
pub fn parse_log(bytes: &[u8]) -> Vec<CommitSummary> {
    bytes
        .split(|&b| b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let f: Vec<&str> = text.trim_matches('\n').split(FIELD_SEP).collect();
            if f.len() < 9 {
                return None;
            }
            Some(CommitSummary {
                id: f[0].to_string(),
                short_id: f[1].to_string(),
                parent_ids: f[2].split_whitespace().map(str::to_string).collect(),
                author_name: f[3].to_string(),
                author_email: f[4].to_string(),
                authored_at: f[5].to_string(),
                committed_at: f[6].to_string(),
                decorations: f[7]
                    .split(", ")
                    .filter(|d| !d.is_empty())
                    .map(str::to_string)
                    .collect(),
                subject: f[8..].join(&FIELD_SEP.to_string()),
            })
        })
        .collect()
}

/// Parse unified diff output for one path into hunks, applying display limits.
pub fn parse_unified_diff(bytes: &[u8], input_truncated: bool, limits: &DiffLimits) -> DiffContent {
    let text = String::from_utf8_lossy(bytes);
    let mut old_path: Option<String> = None;
    let mut new_path: Option<String> = None;
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_no = 0u64;
    let mut new_no = 0u64;
    let mut emitted_lines = 0u64;
    let mut total_lines = 0u64;
    let mut truncated = input_truncated;
    let mut saw_mode_symlink = false;

    for line in text.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            let (ranges, header) = match rest.find(" @@") {
                Some(i) => (&rest[..i], rest[i + 3..].trim().to_string()),
                None => (rest, String::new()),
            };
            let mut it = ranges.split_whitespace();
            let (os, ol) = parse_range(it.next().unwrap_or("-0,0"));
            let (ns, nl) = parse_range(it.next().unwrap_or("+0,0"));
            // Counters increment before each line is emitted, so start one below the range start.
            old_no = os.saturating_sub(1);
            new_no = ns.saturating_sub(1);
            current = Some(Hunk {
                header,
                old_start: os,
                old_lines: ol,
                new_start: ns,
                new_lines: nl,
                lines: Vec::new(),
            });
            continue;
        }
        if current.is_none() {
            // File header section.
            if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
                return DiffContent::NonText {
                    reason: NonTextKind::Binary,
                    summary: "Binary file changed".into(),
                    byte_size: None,
                };
            }
            if line.starts_with("Submodule ") || line.starts_with("Subproject commit") {
                return DiffContent::NonText {
                    reason: NonTextKind::Submodule,
                    summary: line.to_string(),
                    byte_size: None,
                };
            }
            if line.starts_with("new file mode 120000")
                || line.starts_with("old mode 120000")
                || line.starts_with("new mode 120000")
            {
                saw_mode_symlink = true;
            }
            if let Some(p) = line.strip_prefix("--- ") {
                old_path = strip_prefix_ab(p);
            } else if let Some(p) = line.strip_prefix("+++ ") {
                new_path = strip_prefix_ab(p);
            }
            continue;
        }
        let hunk = current.as_mut().expect("hunk in progress");
        let (kind, text_line) = match line.chars().next() {
            Some('+') => (DiffLineKind::Add, &line[1..]),
            Some('-') => (DiffLineKind::Delete, &line[1..]),
            Some(' ') => (DiffLineKind::Context, &line[1..]),
            Some('\\') => continue, // "\ No newline at end of file"
            None => (DiffLineKind::Context, ""),
            _ => continue,
        };
        total_lines += 1;
        let (o, n) = match kind {
            DiffLineKind::Add => {
                new_no += 1;
                (None, Some(new_no))
            }
            DiffLineKind::Delete => {
                old_no += 1;
                (Some(old_no), None)
            }
            DiffLineKind::Context => {
                old_no += 1;
                new_no += 1;
                (Some(old_no), Some(new_no))
            }
        };
        if emitted_lines >= limits.max_lines {
            truncated = true;
            continue;
        }
        emitted_lines += 1;
        hunk.lines.push(DiffLine {
            kind,
            old_no: o,
            new_no: n,
            text: text_line.to_string(),
        });
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if saw_mode_symlink && hunks.iter().all(|h| h.lines.len() <= 2) {
        let target = hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .find(|l| l.kind == DiffLineKind::Add)
            .map(|l| l.text.clone())
            .unwrap_or_default();
        return DiffContent::NonText {
            reason: NonTextKind::Symlink,
            summary: format!("Symbolic link → {target}"),
            byte_size: None,
        };
    }
    if is_lfs_pointer(&hunks) {
        return DiffContent::NonText {
            reason: NonTextKind::LfsPointer,
            summary: "Git LFS pointer changed; the object itself is not downloaded.".into(),
            byte_size: None,
        };
    }
    DiffContent::Text {
        old_path,
        new_path,
        hunks,
        truncated,
        total_lines: Some(total_lines),
    }
}

fn parse_range(s: &str) -> (u64, u64) {
    let s = s.trim_start_matches(['-', '+']);
    let mut it = s.split(',');
    let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let len = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (start, len)
}

fn strip_prefix_ab(p: &str) -> Option<String> {
    let p = p.split('\t').next().unwrap_or(p);
    if p == "/dev/null" {
        return None;
    }
    Some(
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(p)
            .to_string(),
    )
}

fn is_lfs_pointer(hunks: &[Hunk]) -> bool {
    hunks.iter().flat_map(|h| h.lines.iter()).any(|l| {
        l.text
            .starts_with("version https://git-lfs.github.com/spec/")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apple_git_version() {
        assert_eq!(
            parse_version("git version 2.54.0 (Apple Git-157)"),
            Some((2, 54, 0))
        );
        assert_eq!(parse_version("git version 2.11.0"), Some((2, 11, 0)));
        assert_eq!(parse_version("nonsense"), None);
    }

    #[test]
    fn rejects_dangerous_inputs() {
        assert!(validate_revision("--output=x").is_err());
        assert!(validate_revision("HEAD").is_ok());
        assert!(validate_repo_path("../etc/passwd").is_err());
        assert!(validate_repo_path("/abs").is_err());
        assert!(validate_repo_path("src/main.rs").is_ok());
    }

    #[test]
    fn parses_status_with_staged_unstaged_rename_untracked_and_conflict() {
        let raw = concat!(
            "# branch.oid 1111111111111111111111111111111111111111\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
            "1 MM N... 100644 100644 100644 aaaa bbbb both.txt\0",
            "1 .M N... 100644 100644 100644 aaaa aaaa unstaged.txt\0",
            "1 A. N... 000000 100644 100644 0000 cccc new.txt\0",
            "2 R. N... 100644 100644 100644 aaaa aaaa R100 renamed.txt\0old.txt\0",
            "u UU N... 100644 100644 100644 100644 a b c conflict.txt\0",
            "? untracked.txt\0",
            "1 .M S.M. 160000 160000 160000 dddd dddd sub\0",
        );
        let s = parse_status_v2(raw.as_bytes()).unwrap();
        assert_eq!(s.head.kind, HeadKind::Branch);
        assert_eq!(s.head.branch.as_deref(), Some("main"));
        let up = s.upstream.unwrap();
        assert_eq!((up.ahead, up.behind), (2, 1));
        assert_eq!(s.counts.staged, 3); // both.txt, new.txt, renamed.txt
        assert_eq!(s.counts.unstaged, 3); // both.txt, unstaged.txt, sub
        assert_eq!(s.counts.untracked, 1);
        assert_eq!(s.counts.conflicted, 1);
        assert_eq!(s.counts.unique_paths, 7);
        let renamed = s.entries.iter().find(|e| e.path == "renamed.txt").unwrap();
        assert_eq!(renamed.kind, ChangeKind::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some("old.txt"));
        assert!(s.entries.iter().any(|e| e.path == "sub" && e.is_submodule));
    }

    #[test]
    fn parses_unborn_and_detached_heads() {
        let unborn = parse_status_v2(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert_eq!(unborn.head.kind, HeadKind::Unborn);
        let detached = parse_status_v2(b"# branch.oid abc\0# branch.head (detached)\0").unwrap();
        assert_eq!(detached.head.kind, HeadKind::Detached);
        assert_eq!(detached.head.commit_id.as_deref(), Some("abc"));
    }

    #[test]
    fn parses_log_records() {
        let raw = format!(
            "aaaa{s}aaa{s}bbbb cccc{s}Ann{s}ann@x{s}2026-01-01T00:00:00+00:00{s}2026-01-02T00:00:00+00:00{s}HEAD -> main, tag: v1{s}Subject with {s} inside\0",
            s = FIELD_SEP
        );
        let commits = parse_log(raw.as_bytes());
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].parent_ids, vec!["bbbb", "cccc"]);
        assert_eq!(commits[0].decorations, vec!["HEAD -> main", "tag: v1"]);
        assert!(commits[0].subject.starts_with("Subject with"));
    }

    #[test]
    fn parses_unified_diff_with_line_numbers_and_limits() {
        let raw = "diff --git a/f.txt b/f.txt\nindex 1..2 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,4 @@ fn main\n a\n-b\n+B\n+C\n c\n\\ No newline at end of file\n";
        let limits = DiffLimits {
            max_bytes: 1 << 20,
            max_lines: 10_000,
        };
        match parse_unified_diff(raw.as_bytes(), false, &limits) {
            DiffContent::Text {
                hunks,
                truncated,
                old_path,
                new_path,
                ..
            } => {
                assert!(!truncated);
                assert_eq!(old_path.as_deref(), Some("f.txt"));
                assert_eq!(new_path.as_deref(), Some("f.txt"));
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].header, "fn main");
                let l = &hunks[0].lines;
                assert_eq!(l.len(), 5);
                assert_eq!(
                    (l[1].kind, l[1].old_no, l[1].new_no),
                    (DiffLineKind::Delete, Some(2), None)
                );
                assert_eq!(
                    (l[2].kind, l[2].old_no, l[2].new_no),
                    (DiffLineKind::Add, None, Some(2))
                );
                assert_eq!(
                    (l[4].kind, l[4].old_no, l[4].new_no),
                    (DiffLineKind::Context, Some(3), Some(4))
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let tight = DiffLimits {
            max_bytes: 1 << 20,
            max_lines: 2,
        };
        match parse_unified_diff(raw.as_bytes(), false, &tight) {
            DiffContent::Text {
                hunks,
                truncated,
                total_lines,
                ..
            } => {
                assert!(truncated);
                assert_eq!(hunks[0].lines.len(), 2);
                assert_eq!(total_lines, Some(5));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn detects_binary_and_submodule_diffs() {
        let limits = DiffLimits {
            max_bytes: 1 << 20,
            max_lines: 10_000,
        };
        let bin = "diff --git a/x.png b/x.png\nBinary files a/x.png and b/x.png differ\n";
        assert!(matches!(
            parse_unified_diff(bin.as_bytes(), false, &limits),
            DiffContent::NonText {
                reason: NonTextKind::Binary,
                ..
            }
        ));
        let sub = "Submodule sub 1111111..2222222:\n  > change\n";
        assert!(matches!(
            parse_unified_diff(sub.as_bytes(), false, &limits),
            DiffContent::NonText {
                reason: NonTextKind::Submodule,
                ..
            }
        ));
    }
}
