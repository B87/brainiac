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
    ActivityCommit, AppError, AppResult, ChangeCounts, ChangeEntry, ChangeGroup, ChangeKind,
    CommitDetail, CommitFile, CommitSummary, DiffContent, DiffLimits, DiffLine, DiffLineKind,
    DiffOptions, DiffSelector, ErrorCode, HeadKind, HeadState, Hunk, NonTextKind, RefEntry,
    RefKind, StatusSnapshot, UpstreamState,
};

/// Minimum supported Git version (docs/architecture.md, Decisions).
pub const MIN_GIT_VERSION: (u32, u32) = (2, 30);

/// Hard cap on bytes read from any subprocess; larger output is cut and the
/// child is killed. Diff display limits are applied separately and are lower.
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Field and record separators used in custom `git log` formats.
const FIELD_SEP: char = '\u{1f}';

/// First Git version with the `%(ahead-behind:<base>)` ref-filter atom.
const AHEAD_BEHIND_ATOM: (u32, u32) = (2, 41);

/// Configuration that makes a fetch touch nothing but remote-tracking refs,
/// tags, and objects: no automatic cleanup, pruning, commit-graph writes, or
/// hooks (the `reference-transaction` hook runs on ref updates). Per-remote
/// settings such as `remote.<name>.prune` override `-c fetch.prune`, so
/// `FETCH_FLAGS` repeats the pruning choice on the command line. SPEC.md, Fetching.
const FETCH_CONFIG: &[&str] = &[
    "-c",
    "gc.auto=0",
    "-c",
    "maintenance.auto=false",
    "-c",
    "fetch.prune=false",
    "-c",
    "fetch.pruneTags=false",
    "-c",
    "fetch.writeCommitGraph=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "transfer.bundleURI=false",
];

/// Command-line options of every fetch; these beat any configuration. An
/// empty `--refmap` stops Git from also applying the configured
/// `remote.<name>.fetch` refspecs to what it fetched, so only the explicit,
/// checked refspecs decide which refs are written.
const FETCH_FLAGS: &[&str] = &[
    "--refmap=",
    "--no-prune",
    "--no-prune-tags",
    "--no-auto-gc",
    "--no-auto-maintenance",
    "--no-recurse-submodules",
    "--no-write-fetch-head",
    "--quiet",
];

/// Lock files that mean another Git process is changing what a fetch writes
/// (the index and `HEAD` are not among them: a commit can run alongside).
const LOCK_FILES: &[&str] = &[
    "packed-refs.lock",
    "shallow.lock",
    "reftable/tables.list.lock",
];

/// How long a Git process gets to clean up its lock files after SIGTERM.
const TERMINATE_GRACE: Duration = Duration::from_secs(3);

/// What one finished Git process produced.
struct Output {
    stdout: Vec<u8>,
    truncated: bool,
    stderr: String,
    /// Exit code; `None` when a signal ended the process.
    code: Option<i32>,
}

impl Output {
    fn success(&self) -> bool {
        self.code == Some(0)
    }
}

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

/// A registered checkout as the fetcher and the activity tracker see it.
/// Several checkouts (a main checkout and its linked worktrees) can share one
/// Git directory, and with it the refs; `store()` is that shared identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub repository_id: String,
    /// Folder name, for display.
    pub name: String,
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_git_dir: PathBuf,
}

impl Checkout {
    /// Key of the shared Git directory: refs, fetches, and activity belong to it.
    pub fn store(&self) -> String {
        self.common_git_dir.display().to_string()
    }

    /// The main checkout, rather than a linked worktree.
    pub fn is_main(&self) -> bool {
        self.git_dir == self.common_git_dir
    }
}

/// What a full ref name is, given the configured remotes (whose names may
/// contain `/`, so the longest matching remote wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefName<'a> {
    RemoteBranch { remote: &'a str, branch: &'a str },
    Tag(&'a str),
    Other,
}

impl<'a> RefName<'a> {
    /// `'a` ties the returned slices to both the ref name and the remote names.
    pub fn parse(full: &'a str, remotes: &'a [String]) -> Self {
        if let Some(tag) = full.strip_prefix("refs/tags/") {
            return RefName::Tag(tag);
        }
        let Some(rest) = full.strip_prefix("refs/remotes/") else {
            return RefName::Other;
        };
        remotes
            .iter()
            .filter_map(|r| {
                rest.strip_prefix(r.as_str())
                    .and_then(|b| b.strip_prefix('/'))
                    .map(|branch| (r.as_str(), branch))
            })
            .max_by_key(|(r, _)| r.len())
            .filter(|(_, branch)| !branch.is_empty() && *branch != "HEAD")
            .map_or(RefName::Other, |(remote, branch)| RefName::RemoteBranch {
                remote,
                branch,
            })
    }

    /// Short display name: `origin/main` or the tag name.
    pub fn short(full: &str) -> &str {
        full.strip_prefix("refs/remotes/")
            .or_else(|| full.strip_prefix("refs/tags/"))
            .or_else(|| full.strip_prefix("refs/heads/"))
            .unwrap_or(full)
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
            // In a partial clone, Git would otherwise download any object it
            // lacks (a blob for a diff, a commit asked about) from the remote:
            // a write and a network call. Git before 2.44 ignores this.
            .env("GIT_NO_LAZY_FETCH", "1")
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
        let out = self.exec(cwd, args, max_bytes, &[], self.timeout).await?;
        if out.success() || out.truncated {
            Ok((out.stdout, out.truncated))
        } else {
            Err(classify_git_error(args, &out.stderr))
        }
    }

    /// Spawn Git with extra environment variables and a timeout. Only spawn,
    /// read, and timeout failures are errors; a non-zero exit is reported in `Output`.
    async fn exec(
        &self,
        cwd: Option<&Path>,
        args: &[&str],
        max_bytes: usize,
        envs: &[(&str, &str)],
        timeout: Duration,
    ) -> AppResult<Output> {
        let mut cmd = self.command(cwd);
        cmd.args(args);
        for (key, value) in envs {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().map_err(|e| {
            AppError::dependency("Could not start Git.")
                .with_details(format!("{}: {e}", self.binary.display()))
        })?;
        let stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");

        // `move` gives the block the stdout pipe, so stopping early closes it:
        // Git, still writing, gets EPIPE and exits. Keeping the pipe open
        // would leave Git blocked on a full pipe until the timeout.
        let read_all = async move {
            let mut stdout = stdout;
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

        let result = tokio::time::timeout(timeout, async {
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
                terminate(&mut child).await;
                Err(AppError::timeout(format!(
                    "Git did not answer within {} seconds.",
                    timeout.as_secs()
                ))
                .with_details(format!("git {}", args.join(" "))))
            }
            Ok(Err(e)) => {
                Err(AppError::io("Reading Git output failed.").with_details(e.to_string()))
            }
            Ok(Ok((stdout, truncated, err, status))) => Ok(Output {
                stdout,
                truncated,
                stderr: String::from_utf8_lossy(&err).trim().to_string(),
                code: status.code(),
            }),
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
        // Git prints relative metadata paths (such as `--git-common-dir` from a
        // subfolder, `../.git`) relative to the directory it ran in, not the root.
        let git_dir = canonicalize(&absolute_in(path, git_dir)).await?;
        let common_git_dir = canonicalize(&absolute_in(path, common)).await?;
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
        parse_status_v2(&out)
    }

    /// Committer date of a commit, RFC 3339.
    pub async fn commit_time(&self, root: &Path, commit_id: &str) -> Option<String> {
        validate_revision(commit_id).ok()?;
        let out = self
            .run_raw(
                Some(root),
                &[
                    "log",
                    "-1",
                    "--format=%cI",
                    "--end-of-options",
                    commit_id,
                    "--",
                ],
            )
            .await
            .ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Whether a ref pattern is valid for Git: `refs/heads/<p>` as a fetch
    /// refspec pattern (one `*` at most) for branches, `refs/tags/<p>` with
    /// each `*` standing for a name character for tags (which are only matched).
    pub async fn ref_pattern_ok(&self, pattern: &str, branch: bool) -> bool {
        if pattern.starts_with('-') {
            return false;
        }
        let (args, full) = if branch {
            (
                vec!["check-ref-format", "--refspec-pattern"],
                format!("refs/heads/{pattern}"),
            )
        } else {
            (
                vec!["check-ref-format"],
                format!("refs/tags/{}", pattern.replace('*', "x")),
            )
        };
        let mut args = args;
        args.push(&full);
        self.exec(None, &args, 1024, &[], self.timeout)
            .await
            .is_ok_and(|out| out.success())
    }

    /// Whether `id` names an object in the repository. Errors other than
    /// "missing" (a timeout, a failed spawn) are errors.
    pub async fn object_exists(&self, root: &Path, id: &str) -> AppResult<bool> {
        validate_revision(id)?;
        let args = ["cat-file", "-e", "--end-of-options", id];
        let out = self
            .exec(Some(root), &args, 1024, &[], self.timeout)
            .await?;
        Ok(out.success())
    }

    /// Whether the repository has any of the commits (or tags) `ids`, in one
    /// Git run: `--ignore-missing` lists the ones it has and skips the rest.
    /// `None` when Git cannot tell without downloading: a partial clone with a
    /// Git too old to honor `GIT_NO_LAZY_FETCH`.
    pub async fn contains_any(&self, root: &Path, ids: &[String]) -> AppResult<Option<bool>> {
        if ids.is_empty() {
            return Ok(Some(false));
        }
        let honors_no_lazy_fetch =
            parse_version(&self.version).is_some_and(|(major, minor, _)| (major, minor) >= (2, 44));
        if !honors_no_lazy_fetch
            && self
                .config_value(root, "extensions.partialClone")
                .await
                .is_some()
        {
            return Ok(None);
        }
        let mut args = vec![
            "rev-list",
            "--no-walk",
            "--ignore-missing",
            "--end-of-options",
        ];
        for id in ids {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(AppError::validation("Malformed object ID."));
            }
            args.push(id);
        }
        let out = self.run_raw(Some(root), &args).await?;
        Ok(Some(!out.iter().all(u8::is_ascii_whitespace)))
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
        query: &LogQuery<'_>,
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
        let author;
        let exclude;
        let filter = query.filter.map(str::trim).filter(|f| !f.is_empty());
        let by = query.author.map(str::trim).filter(|a| !a.is_empty());
        if filter.is_some() || by.is_some() {
            args.extend(["--regexp-ignore-case", "--fixed-strings"]);
        }
        if let Some(f) = filter {
            grep = format!("--grep={f}");
            args.push(&grep);
        }
        if let Some(a) = by {
            author = format!("--author={a}");
            args.push(&author);
        }
        args.extend(["--end-of-options", anchor]);
        if let Some(x) = query.exclude {
            validate_revision(x)?;
            exclude = format!("^{x}");
            args.push(&exclude);
        }
        args.push("--");
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
        self.log(root, &id, 0, 1, &LogQuery::default())
            .await
            .map(|mut v| {
                v.truncate(1);
                v
            })
    }

    /// Full parent ids of a commit, in order (empty for a root commit).
    async fn commit_parents(&self, root: &Path, commit_id: &str) -> AppResult<Vec<String>> {
        validate_revision(commit_id)?;
        let spec = format!("{commit_id}^{{commit}}");
        // Prints "<commit> <parent1> <parent2> ...".
        let out = self
            .run_raw(
                Some(root),
                &[
                    "rev-list",
                    "--parents",
                    "--max-count=1",
                    "--end-of-options",
                    &spec,
                    "--",
                ],
            )
            .await?;
        Ok(String::from_utf8_lossy(&out)
            .split_whitespace()
            .skip(1)
            .map(str::to_string)
            .collect())
    }

    /// Metadata, full message, and changed files of one commit, compared with
    /// the parent at `parent_index` (the empty tree for a root commit).
    /// `repository_id` is left empty for the caller to fill in.
    pub async fn commit_detail(
        &self,
        root: &Path,
        commit_id: &str,
        parent_index: u32,
    ) -> AppResult<CommitDetail> {
        validate_revision(commit_id)?;
        let format = format!(
            "--format=%H{s}%h{s}%P{s}%an{s}%ae{s}%aI{s}%cI{s}%D{s}%cn{s}%ce{s}%s{s}%b",
            s = FIELD_SEP
        );
        let spec = format!("{commit_id}^{{commit}}");
        let out = self
            .run_raw(
                Some(root),
                &[
                    "log",
                    "-z",
                    "--max-count=1",
                    &format,
                    "--end-of-options",
                    &spec,
                    "--",
                ],
            )
            .await?;
        let mut detail = parse_commit_header(&out)
            .ok_or_else(|| AppError::not_found(format!("Commit {commit_id} was not found.")))?;

        let base = compare_base(&detail.summary.parent_ids, parent_index)?;
        let id = detail.summary.id.clone();
        let mut revs: Vec<&str> = Vec::new();
        match base {
            Some(parent) => revs.extend(["--end-of-options", parent, &id]),
            None => revs.extend(["--root", "--end-of-options", &id]),
        }
        let tree_diff = |format: &'static str| {
            let mut args = vec![
                "diff-tree",
                "-r",
                "-z",
                "--no-commit-id",
                "--find-renames",
                format,
            ];
            args.extend(revs.iter().copied());
            args
        };
        let names = self
            .run_raw(Some(root), &tree_diff("--name-status"))
            .await?;
        let stats = self.run_raw(Some(root), &tree_diff("--numstat")).await?;
        detail.files = merge_file_stats(parse_name_status(&names), &parse_numstat(&stats));
        detail.compared_parent_index = parent_index;
        Ok(detail)
    }

    // -----------------------------------------------------------------------
    // Refs
    // -----------------------------------------------------------------------

    /// Local branches, remote-tracking branches, and tags, sorted by full name,
    /// plus the default branch they are compared with (full name), if any.
    pub async fn list_refs_with_base(
        &self,
        root: &Path,
    ) -> AppResult<(Vec<RefEntry>, Option<String>)> {
        let mut refs = self.list_refs(root).await?;
        let Some(base) = self.default_base(root, &refs).await else {
            return Ok((refs, None));
        };
        let counts = self.base_counts(root, &refs, &base).await?;
        for r in &mut refs {
            if r.kind == RefKind::Tag || r.full_name == base {
                continue;
            }
            if let Some(&(ahead, behind)) = counts.get(&r.full_name) {
                r.base_ahead = Some(ahead);
                r.base_behind = Some(behind);
            }
        }
        Ok((refs, Some(base)))
    }

    /// The repository's default branch: the target of `<remote>/HEAD`, else the
    /// remote's `main` or `master`, else a local `main` or `master`. The remote
    /// is `origin` when it exists, else the first one listed.
    async fn default_base(&self, root: &Path, refs: &[RefEntry]) -> Option<String> {
        let has = |name: &str| refs.iter().any(|r| r.full_name == name);
        // Ask Git for the remote names: they may contain `/`, so they cannot
        // be read off the ref names.
        let remotes = self.remotes(root).await.unwrap_or_default();
        let remote = remotes
            .iter()
            .find(|r| *r == "origin")
            .or_else(|| remotes.first())
            .cloned();
        if let Some(remote) = &remote {
            let head = format!("refs/remotes/{remote}/HEAD");
            if let Ok(out) = self
                .run_raw(Some(root), &["symbolic-ref", "--quiet", &head])
                .await
            {
                let target = String::from_utf8_lossy(&out).trim().to_string();
                if has(&target) {
                    return Some(target);
                }
            }
            for name in ["main", "master"] {
                let full = format!("refs/remotes/{remote}/{name}");
                if has(&full) {
                    return Some(full);
                }
            }
        }
        ["refs/heads/main", "refs/heads/master"]
            .into_iter()
            .find(|n| has(n))
            .map(str::to_string)
    }

    /// Ahead/behind counts of every branch against `base`, keyed by full name.
    async fn base_counts(
        &self,
        root: &Path,
        refs: &[RefEntry],
        base: &str,
    ) -> AppResult<std::collections::HashMap<String, (u32, u32)>> {
        let mut counts = std::collections::HashMap::new();
        let modern = parse_version(&self.version)
            .is_some_and(|(major, minor, _)| (major, minor) >= AHEAD_BEHIND_ATOM);
        if modern {
            let format = format!("--format=%(refname)%1f%(ahead-behind:{base})");
            let out = self
                .run_raw(
                    Some(root),
                    &["for-each-ref", &format, "refs/heads", "refs/remotes"],
                )
                .await?;
            for line in String::from_utf8_lossy(&out).lines() {
                if let Some((name, ab)) = line.split_once(FIELD_SEP) {
                    let mut it = ab.split_whitespace().map(|n| n.parse::<u32>().ok());
                    if let (Some(Some(a)), Some(Some(b))) = (it.next(), it.next()) {
                        counts.insert(name.to_string(), (a, b));
                    }
                }
            }
        } else {
            // Older Git: one `rev-list` per local branch, capped to keep the tab responsive.
            for r in refs
                .iter()
                .filter(|r| r.kind == RefKind::LocalBranch && r.full_name != base)
                .take(100)
            {
                let range = format!("{}...{base}", r.full_name);
                let out = self
                    .run_raw(
                        Some(root),
                        &["rev-list", "--left-right", "--count", &range, "--"],
                    )
                    .await?;
                let text = String::from_utf8_lossy(&out);
                let mut it = text.split_whitespace().map(|n| n.parse::<u32>().ok());
                if let (Some(Some(a)), Some(Some(b))) = (it.next(), it.next()) {
                    counts.insert(r.full_name.clone(), (a, b));
                }
            }
        }
        Ok(counts)
    }

    /// Local branches, remote-tracking branches, and tags, sorted by full name.
    pub async fn list_refs(&self, root: &Path) -> AppResult<Vec<RefEntry>> {
        // `%1f` is for-each-ref's hex escape for the field separator.
        // Fields 7 to 11 describe the tip commit; the `*` variants peel annotated tags.
        let format = "--format=%(refname)%1f%(refname:short)%1f%(objectname)%1f%(*objectname)%1f%(HEAD)%1f%(upstream:short)%1f%(symref)\
            %1f%(subject)%1f%(*subject)%1f%(committerdate:iso-strict)%1f%(*committerdate:iso-strict)%1f%(upstream:track,nobracket)";
        let out = self
            .run_raw(
                Some(root),
                &[
                    "for-each-ref",
                    format,
                    "refs/heads",
                    "refs/remotes",
                    "refs/tags",
                ],
            )
            .await?;
        Ok(parse_refs(&out))
    }

    // -----------------------------------------------------------------------
    // Diffs
    // -----------------------------------------------------------------------

    pub async fn diff(
        &self,
        root: &Path,
        selector: &DiffSelector,
        limits: &DiffLimits,
        options: DiffOptions,
    ) -> AppResult<DiffContent> {
        let path = selector.path();
        validate_repo_path(path)?;
        let max_bytes = limits.max_bytes as usize;
        let mut common = vec![
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
            "--find-renames",
        ];
        if options.ignore_whitespace {
            common.push("--ignore-all-space");
        }
        let (raw, truncated) = match selector {
            DiffSelector::WorktreeVsIndex { .. } => {
                let mut args = vec!["diff"];
                args.extend(common.iter().copied());
                args.extend(["--", path]);
                self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                    .await?
            }
            DiffSelector::IndexVsHead { .. } => {
                let mut args = vec!["diff", "--cached"];
                args.extend(common.iter().copied());
                args.extend(["--", path]);
                self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                    .await?
            }
            DiffSelector::WorktreeVsHead { .. } => {
                let mut args = vec!["diff"];
                args.extend(common.iter().copied());
                args.extend(["HEAD", "--", path]);
                self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                    .await?
            }
            DiffSelector::UntrackedPreview { .. } => {
                return self.untracked_preview(root, path, limits).await;
            }
            DiffSelector::Commit {
                commit_id,
                old_path,
                parent_index,
                ..
            } => {
                validate_revision(commit_id)?;
                let parents = self.commit_parents(root, commit_id).await?;
                let base = compare_base(&parents, *parent_index)?;
                let mut args = vec!["diff-tree", "-r", "-p", "--no-commit-id"];
                args.extend(common.iter().copied());
                // `--root` compares a root commit with the empty tree.
                match base {
                    Some(parent) => args.extend(["--end-of-options", parent, commit_id]),
                    None => args.extend(["--root", "--end-of-options", commit_id]),
                }
                args.extend(["--", path]);
                // Naming the old path too lets rename detection pair the two sides.
                if let Some(old) = old_path.as_deref().filter(|o| *o != path) {
                    validate_repo_path(old)?;
                    args.push(old);
                }
                self.run_bounded(Some(root), &args, max_bytes.saturating_mul(2))
                    .await?
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

    // -----------------------------------------------------------------------
    // Working-tree line counts
    // -----------------------------------------------------------------------

    /// `--numstat` for the index (`staged`) and the working tree (`unstaged`),
    /// keyed by path. Binary files map to `(None, None)`.
    pub async fn change_stats(&self, root: &Path) -> AppResult<(NumStats, NumStats)> {
        let base = [
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--numstat",
            "-z",
            "--find-renames",
        ];
        let mut staged = vec!["diff", "--cached"];
        staged.extend(base);
        let mut unstaged = vec!["diff"];
        unstaged.extend(base);
        let staged = self.run_raw(Some(root), &staged).await?;
        let unstaged = self.run_raw(Some(root), &unstaged).await?;
        Ok((parse_numstat(&staged), parse_numstat(&unstaged)))
    }

    // -----------------------------------------------------------------------
    // Fetching (SPEC.md, Fetching): the only command that writes to a repository
    // -----------------------------------------------------------------------

    /// Names of the configured remotes.
    pub async fn remotes(&self, root: &Path) -> AppResult<Vec<String>> {
        let out = self.run_raw(Some(root), &["remote"]).await?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// The checked-out branch's short name; `None` for a detached HEAD.
    pub async fn head_branch(&self, root: &Path) -> Option<String> {
        let out = self
            .run_raw(Some(root), &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await
            .ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_string()).filter(|b| !b.is_empty())
    }

    /// One configuration value, or `None` when it is not set.
    pub async fn config_value(&self, root: &Path, key: &str) -> Option<String> {
        let out = self
            .run_raw(Some(root), &["config", "--get", key])
            .await
            .ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_string()).filter(|v| !v.is_empty())
    }

    /// The remote's configured fetch refspecs that only write remote-tracking
    /// refs or tags. Anything else (a mirror refspec such as
    /// `+refs/heads/*:refs/heads/*` would rewrite local branches) is dropped;
    /// when nothing usable is left, the standard mapping is used.
    pub async fn safe_fetch_refspecs(&self, root: &Path, remote: &str) -> AppResult<Vec<String>> {
        validate_revision(remote)?;
        let key = format!("remote.{remote}.fetch");
        let configured = match self
            .run_raw(Some(root), &["config", "--get-all", &key])
            .await
        {
            Ok(out) => String::from_utf8_lossy(&out)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            // `git config` exits 1 when the key is not set.
            Err(_) => Vec::new(),
        };
        let mut safe: Vec<String> = configured
            .into_iter()
            .filter(|r| refspec_is_safe(r, remote))
            .collect();
        if !safe.iter().any(|r| !r.starts_with('^')) {
            safe = vec![format!("+refs/heads/*:refs/remotes/{remote}/*")];
        }
        Ok(safe)
    }

    /// Fetch `remote` with the hardened invocation and explicit refspecs, each
    /// of which must write only remote-tracking refs or tags. Prompts are
    /// impossible: Git cannot ask for a password, Git Credential Manager is
    /// told not to open windows, and SSH runs in batch mode unless the user
    /// configured their own `core.sshCommand`. A refspec for a branch the remote
    /// no longer has would fail the whole fetch, so it is dropped and the fetch
    /// retried; the dropped sources are returned.
    pub async fn fetch(
        &self,
        root: &Path,
        remote: &str,
        refspecs: &[String],
        timeout: Duration,
    ) -> AppResult<Vec<String>> {
        validate_revision(remote)?;
        if refspecs.is_empty() {
            return Err(AppError::validation("Nothing to fetch."));
        }
        for r in refspecs {
            if !refspec_is_safe(r, remote) {
                return Err(AppError::validation(format!(
                    "Refusing a refspec that could change more than remote-tracking refs: {r:?}"
                )));
            }
        }
        let custom_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some()
            || std::env::var_os("GIT_SSH").is_some()
            || self.config_value(root, "core.sshCommand").await.is_some();
        let mut envs: Vec<(&str, &str)> = vec![
            ("GIT_ASKPASS", ""),
            ("SSH_ASKPASS", ""),
            ("GCM_INTERACTIVE", "never"),
        ];
        if !custom_ssh {
            envs.push((
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o ConnectTimeout=15",
            ));
        }
        let mut specs: Vec<String> = refspecs.to_vec();
        let mut skipped = Vec::new();
        loop {
            let mut args: Vec<&str> = FETCH_CONFIG.to_vec();
            args.push("fetch");
            args.extend(FETCH_FLAGS);
            args.push(remote);
            args.extend(specs.iter().map(String::as_str));
            let out = self
                .exec(Some(root), &args, MAX_OUTPUT_BYTES, &envs, timeout)
                .await?;
            if out.success() {
                return Ok(skipped);
            }
            // Only plain names can be missing; globs simply match nothing.
            let missing = missing_remote_ref(&out.stderr);
            let before = specs.len();
            if let Some(gone) = &missing {
                specs.retain(|s| refspec_source(s) != Some(gone.as_str()));
            }
            if missing.is_none() || specs.len() == before {
                return Err(classify_fetch_error(remote, &out.stderr));
            }
            skipped.extend(missing);
            if specs.iter().all(|s| s.starts_with('^')) {
                return Ok(skipped);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Activity (SPEC.md, Workspace activity)
    // -----------------------------------------------------------------------

    /// Tips of remote-tracking branches and tags, annotated tags peeled to
    /// their commit. Symbolic refs are skipped.
    pub async fn remote_and_tag_tips(&self, root: &Path) -> AppResult<Vec<RefTip>> {
        let out = self
            .run_raw(
                Some(root),
                &[
                    "for-each-ref",
                    "--format=%(refname)%1f%(objectname)%1f%(*objectname)%1f%(symref)%1f%(creatordate:unix)",
                    "refs/remotes",
                    "refs/tags",
                ],
            )
            .await?;
        Ok(parse_tips(&out))
    }

    /// Whether `ancestor` is reachable from `descendant`. Git exits 1 for
    /// "no"; any other failure (such as a commit that no longer exists) is an
    /// error, not a "no".
    pub async fn is_ancestor(
        &self,
        root: &Path,
        ancestor: &str,
        descendant: &str,
    ) -> AppResult<bool> {
        validate_revision(ancestor)?;
        validate_revision(descendant)?;
        let args = [
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            ancestor,
            descendant,
        ];
        let out = self
            .exec(Some(root), &args, MAX_OUTPUT_BYTES, &[], self.timeout)
            .await?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(classify_git_error(&args, &out.stderr)),
        }
    }

    /// Number of commits reachable from `include` but from none of `exclude`.
    pub async fn count_commits(
        &self,
        root: &Path,
        include: &str,
        exclude: &[&str],
    ) -> AppResult<u32> {
        let revs = range_args(include, exclude)?;
        let mut args = vec!["rev-list", "--count", "--end-of-options"];
        args.extend(revs.iter().map(String::as_str));
        args.push("--");
        let out = self.run_raw(Some(root), &args).await?;
        Ok(String::from_utf8_lossy(&out).trim().parse().unwrap_or(0))
    }

    /// Newest commits of a range, at most `limit`.
    pub async fn range_commits(
        &self,
        root: &Path,
        include: &str,
        exclude: &[&str],
        limit: u32,
    ) -> AppResult<Vec<ActivityCommit>> {
        let revs = range_args(include, exclude)?;
        let format = format!("--format=%H{s}%h{s}%P{s}%an{s}%cI{s}%s", s = FIELD_SEP);
        let count = format!("--max-count={limit}");
        let mut args = vec!["log", "-z", &format, &count, "--end-of-options"];
        args.extend(revs.iter().map(String::as_str));
        args.push("--");
        let out = self.run_raw(Some(root), &args).await?;
        Ok(parse_activity_commits(&out))
    }

    /// Commit count, merge count, and authors (most commits first) of a range,
    /// looking at no more than 5,000 commits.
    pub async fn range_stats(
        &self,
        root: &Path,
        include: &str,
        exclude: &[&str],
    ) -> AppResult<RangeStats> {
        let revs = range_args(include, exclude)?;
        let format = format!("--format=%an{FIELD_SEP}%P");
        let mut args = vec!["log", &format, "--max-count=5000", "--end-of-options"];
        args.extend(revs.iter().map(String::as_str));
        args.push("--");
        let out = self.run_raw(Some(root), &args).await?;
        Ok(parse_range_stats(&out))
    }

    /// Paths that differ between two commits, at most `limit`.
    pub async fn changed_paths(
        &self,
        root: &Path,
        old: &str,
        new: &str,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        validate_revision(old)?;
        validate_revision(new)?;
        let (out, _) = self
            .run_bounded(
                Some(root),
                &[
                    "diff",
                    "--name-only",
                    "-z",
                    "--no-renames",
                    "--no-ext-diff",
                    "--end-of-options",
                    old,
                    new,
                    "--",
                ],
                1024 * 1024,
            )
            .await?;
        Ok(out
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .take(limit)
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect())
    }

    /// The newest tag reachable from the parent of `commit`, if any.
    pub async fn previous_tag(&self, root: &Path, commit: &str) -> Option<String> {
        validate_revision(commit).ok()?;
        let parent = format!("{commit}^");
        let out = self
            .run_raw(
                Some(root),
                &[
                    "describe",
                    "--tags",
                    "--abbrev=0",
                    "--end-of-options",
                    &parent,
                ],
            )
            .await
            .ok()?;
        Some(String::from_utf8_lossy(&out).trim().to_string()).filter(|t| !t.is_empty())
    }

    /// Author name, whether it is a merge, and the commit time (Unix seconds),
    /// for every commit reachable from `refs` and committed after `since`
    /// (at most 10,000).
    pub async fn commits_since(
        &self,
        root: &Path,
        refs: &[String],
        since: &str,
    ) -> AppResult<Vec<(String, bool, i64)>> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        for r in refs {
            validate_revision(r)?;
        }
        let since = format!("--since={since}");
        let format = format!("--format=%an{FIELD_SEP}%P{FIELD_SEP}%ct");
        let mut args = vec![
            "log",
            &format,
            &since,
            "--max-count=10000",
            "--end-of-options",
        ];
        args.extend(refs.iter().map(String::as_str));
        args.push("--");
        let out = self.run_raw(Some(root), &args).await?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| {
                let mut f = l.split(FIELD_SEP);
                let author = f.next()?;
                let parents = f.next()?;
                let time = f.next()?.trim().parse().ok()?;
                Some((
                    author.to_string(),
                    parents.split_whitespace().count() > 1,
                    time,
                ))
            })
            .collect())
    }
}

/// `path -> (additions, deletions)`; both `None` for a binary file.
pub type NumStats = std::collections::HashMap<String, (Option<u64>, Option<u64>)>;

/// Filters for `GitService::log`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogQuery<'a> {
    /// Substring of the message (case-insensitive).
    pub filter: Option<&'a str>,
    /// Substring of the author's name or email (case-insensitive).
    pub author: Option<&'a str>,
    /// Revision whose history is left out.
    pub exclude: Option<&'a str>,
}

/// Commit counts and authors of a commit range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RangeStats {
    pub commits: u32,
    pub merges: u32,
    /// Most commits first.
    pub authors: Vec<String>,
}

/// `include` followed by `^exclude` for each excluded revision, validated.
fn range_args(include: &str, exclude: &[&str]) -> AppResult<Vec<String>> {
    validate_revision(include)?;
    let mut revs = vec![include.to_string()];
    for x in exclude {
        validate_revision(x)?;
        revs.push(format!("^{x}"));
    }
    Ok(revs)
}

/// Whether a refspec writes only remote-tracking refs of `remote` or tags.
/// Negative refspecs (`^refs/heads/x`) only exclude and are always safe.
pub fn refspec_is_safe(spec: &str, remote: &str) -> bool {
    if spec.contains(['\0', '\n', ' ']) || spec.starts_with('-') {
        return false;
    }
    if let Some(neg) = spec.strip_prefix('^') {
        return neg.starts_with("refs/") && !neg.contains(':');
    }
    let forced = spec.starts_with('+');
    let spec = spec.strip_prefix('+').unwrap_or(spec);
    let Some((src, dst)) = spec.split_once(':') else {
        // No destination: only FETCH_HEAD, which `--no-write-fetch-head` skips.
        return false;
    };
    if src.is_empty() || dst.contains("..") {
        return false;
    }
    // Tags: only a tag into the same name, never forced, so a fetch cannot
    // overwrite the user's own tags.
    if dst.starts_with("refs/tags/") {
        return !forced && src == dst;
    }
    dst.starts_with(&format!("refs/remotes/{remote}/"))
}

/// The source side of a refspec, without `+`: `refs/heads/main`.
fn refspec_source(spec: &str) -> Option<&str> {
    let spec = spec.strip_prefix('+').unwrap_or(spec);
    spec.split_once(':').map(|(src, _)| src)
}

/// The ref named by "couldn't find remote ref X", as a full `refs/heads/` name.
fn missing_remote_ref(stderr: &str) -> Option<String> {
    let rest = stderr
        .lines()
        .find_map(|l| l.split_once("couldn't find remote ref ").map(|(_, r)| r))?;
    let name = rest.trim();
    Some(if name.starts_with("refs/") {
        name.to_string()
    } else {
        format!("refs/heads/{name}")
    })
}

/// The lock file that shows another Git process is changing the repository,
/// if any (SPEC.md, Fetching).
pub fn busy_lock(git_dir: &Path, common_git_dir: &Path, remote: &str) -> Option<PathBuf> {
    for dir in [git_dir, common_git_dir] {
        for name in LOCK_FILES {
            let p = dir.join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    let refs = common_git_dir.join("refs");
    find_lock(&refs.join("remotes").join(remote), 12).or_else(|| find_lock(&refs.join("tags"), 12))
}

fn find_lock(dir: &Path, depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "lock") {
            return Some(path);
        }
        if depth > 0 && path.is_dir() {
            if let Some(found) = find_lock(&path, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// Turn a failed fetch into an error that names the recovery step. Patterns
/// are whole phrases Git prints, so a "401" inside a URL is not mistaken for
/// an HTTP status.
pub fn classify_fetch_error(remote: &str, stderr: &str) -> AppError {
    let lower = stderr.to_ascii_lowercase();
    let details = format!("git fetch {remote}\n{stderr}");
    let any = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    let lock = lower.contains("cannot lock ref")
        || (lower.contains("unable to create '") && lower.contains(".lock'"));
    if any(&[
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "authentication failed",
        "permission denied (publickey",
        "host key verification failed",
        "repository not found",
        "returned error: 401",
        "returned error: 403",
        "access denied",
    ]) {
        AppError::new(
            ErrorCode::PermissionDenied,
            format!("Fetching from {remote} needs sign-in. Fetch once from a terminal or your editor so Git can store the credentials, then try again."),
        )
        .with_details(details)
    } else if lock {
        AppError::new(
            ErrorCode::Conflict,
            "Another Git command is using this repository. Try again in a moment.",
        )
        .with_details(details)
    } else if any(&[
        "could not resolve host",
        "network is unreachable",
        "connection timed out",
        "operation timed out",
        "connection refused",
        "unable to access",
        "could not read from remote repository",
    ]) {
        AppError::io(format!(
            "Could not reach {remote}. Check the network or VPN, then try again."
        ))
        .with_details(details)
    } else if any(&["does not appear to be a git repository", "no such remote"]) {
        AppError::validation(format!("{remote} is not a usable remote.")).with_details(details)
    } else {
        AppError::io(format!("Fetching from {remote} failed.")).with_details(details)
    }
}

/// One remote-tracking branch or tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefTip {
    /// Full ref name.
    pub name: String,
    /// The commit it points at (annotated tags peeled).
    pub target: String,
    /// When it was made, Unix seconds: the tag's date, or the tip commit's.
    pub time: i64,
}

/// Parse the `remote_and_tag_tips` format.
pub fn parse_tips(bytes: &[u8]) -> Vec<RefTip> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(FIELD_SEP).collect();
            if f.len() < 4 || !f[3].is_empty() {
                return None;
            }
            let target = if f[2].is_empty() { f[1] } else { f[2] };
            Some(RefTip {
                name: f[0].to_string(),
                target: target.to_string(),
                time: f.get(4).and_then(|t| t.trim().parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

/// Parse `git log -z` in the `range_commits` format.
pub fn parse_activity_commits(bytes: &[u8]) -> Vec<ActivityCommit> {
    bytes
        .split(|&b| b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let f: Vec<&str> = text.trim_matches('\n').split(FIELD_SEP).collect();
            if f.len() < 6 {
                return None;
            }
            Some(ActivityCommit {
                id: f[0].to_string(),
                short_id: f[1].to_string(),
                is_merge: f[2].split_whitespace().count() > 1,
                author_name: f[3].to_string(),
                committed_at: f[4].to_string(),
                subject: f[5..].join(&FIELD_SEP.to_string()),
            })
        })
        .collect()
}

/// Parse `git log --format=%an<US>%P` into counts and authors by commit count.
pub fn parse_range_stats(bytes: &[u8]) -> RangeStats {
    let mut stats = RangeStats::default();
    let mut by_author: Vec<(String, u32)> = Vec::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let Some((author, parents)) = line.split_once(FIELD_SEP) else {
            continue;
        };
        stats.commits += 1;
        if parents.split_whitespace().count() > 1 {
            stats.merges += 1;
        }
        match by_author.iter_mut().find(|(a, _)| a == author) {
            Some((_, n)) => *n += 1,
            None => by_author.push((author.to_string(), 1)),
        }
    }
    // A stable sort keeps first-seen (newest) order among equal counts.
    by_author.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    stats.authors = by_author.into_iter().map(|(a, _)| a).collect();
    stats
}

// ---------------------------------------------------------------------------
// Helpers and parsers (pure functions; unit-tested below)
// ---------------------------------------------------------------------------

/// Stop a Git process that ran out of time. SIGTERM first, which Git catches
/// to remove its lock files; SIGKILL only if it is still running after a grace
/// period. A SIGKILLed fetch could leave `*.lock` files that block the user's
/// own Git commands.
async fn terminate(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if tokio::time::timeout(TERMINATE_GRACE, child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }
    let _ = child.kill().await;
}

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
                        additions: None,
                        deletions: None,
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
                        additions: None,
                        deletions: None,
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
                        additions: None,
                        deletions: None,
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
                    additions: None,
                    deletions: None,
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
            } else if let Some(p) = line.strip_prefix("rename from ") {
                // A pure rename has no ---/+++ lines; these headers name both sides.
                old_path = Some(p.to_string());
            } else if let Some(p) = line.strip_prefix("rename to ") {
                new_path = Some(p.to_string());
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

/// The parent to compare a commit with: `None` means the empty tree (root commit).
pub fn compare_base(parents: &[String], parent_index: u32) -> AppResult<Option<&str>> {
    match parents.get(parent_index as usize) {
        Some(parent) => Ok(Some(parent.as_str())),
        None if parents.is_empty() && parent_index == 0 => Ok(None),
        None => Err(AppError::validation(format!(
            "This commit has {} parent(s); parent {} does not exist.",
            parents.len(),
            parent_index + 1
        ))),
    }
}

/// Parse one `git log -z` record in the commit-detail format (summary fields,
/// then committer name/email, subject, and body).
pub fn parse_commit_header(bytes: &[u8]) -> Option<CommitDetail> {
    let record = bytes.split(|&b| b == 0).find(|r| !r.is_empty())?;
    let text = String::from_utf8_lossy(record);
    let f: Vec<&str> = text.split(FIELD_SEP).collect();
    if f.len() < 12 {
        return None;
    }
    Some(CommitDetail {
        repository_id: String::new(),
        summary: CommitSummary {
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
            subject: f[10].to_string(),
        },
        committer_name: f[8].to_string(),
        committer_email: f[9].to_string(),
        body: f[11..].join(&FIELD_SEP.to_string()).trim_end().to_string(),
        compared_parent_index: 0,
        files: Vec::new(),
    })
}

/// Parse `git diff-tree -r -z --name-status`: `STATUS\0path\0`, or
/// `R<score>\0old\0new\0` for renames and copies.
pub fn parse_name_status(bytes: &[u8]) -> Vec<CommitFile> {
    let mut fields = bytes
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let Some(code) = status.chars().next() else {
            continue;
        };
        let Some(first) = fields.next() else { break };
        let (path, old_path) = if matches!(code, 'R' | 'C') {
            match fields.next() {
                Some(new) => (new, Some(first)),
                None => break,
            }
        } else {
            (first, None)
        };
        files.push(CommitFile {
            path,
            old_path,
            kind: kind_from_char(code),
            additions: None,
            deletions: None,
            is_binary: false,
        });
    }
    files
}

/// Parse `git diff-tree -r -z --numstat` into `path -> (additions, deletions)`.
/// Binary files report `-` and map to `None`. Renames are
/// `adds\tdels\t\0old\0new\0` and are keyed by the new path.
pub fn parse_numstat(
    bytes: &[u8],
) -> std::collections::HashMap<String, (Option<u64>, Option<u64>)> {
    let mut stats = std::collections::HashMap::new();
    let mut fields = bytes
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let adds = parts.next().and_then(|v| v.parse().ok());
        let dels = parts.next().and_then(|v| v.parse().ok());
        let path = match parts.next() {
            Some(p) if !p.is_empty() => p.to_string(),
            // Empty path field: a rename, followed by the old and new paths.
            _ => {
                let _old = fields.next();
                match fields.next() {
                    Some(new) => new,
                    None => break,
                }
            }
        };
        stats.insert(path, (adds, dels));
    }
    stats
}

fn merge_file_stats(
    mut files: Vec<CommitFile>,
    stats: &std::collections::HashMap<String, (Option<u64>, Option<u64>)>,
) -> Vec<CommitFile> {
    for file in &mut files {
        if let Some(&(adds, dels)) = stats.get(&file.path) {
            file.additions = adds;
            file.deletions = dels;
            file.is_binary = adds.is_none() && dels.is_none();
        }
    }
    files
}

/// Parse `git for-each-ref` output in the `list_refs` format.
pub fn parse_refs(bytes: &[u8]) -> Vec<RefEntry> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(FIELD_SEP).collect();
            if f.len() < 7 || !f[6].is_empty() {
                // Too short, or a symbolic ref such as `origin/HEAD`.
                return None;
            }
            let kind = if f[0].starts_with("refs/heads/") {
                RefKind::LocalBranch
            } else if f[0].starts_with("refs/remotes/") {
                RefKind::RemoteBranch
            } else if f[0].starts_with("refs/tags/") {
                RefKind::Tag
            } else {
                return None;
            };
            let field = |i: usize| f.get(i).copied().unwrap_or("");
            // Annotated tags point at a tag object; the `*` field is the commit behind it.
            let peeled = !f[3].is_empty();
            let upstream = Some(f[5].to_string()).filter(|u| !u.is_empty());
            let (ahead, behind) = match &upstream {
                Some(_) => parse_track(field(11)),
                None => (None, None),
            };
            Some(RefEntry {
                name: f[1].to_string(),
                full_name: f[0].to_string(),
                kind,
                target_id: if peeled { f[3] } else { f[2] }.to_string(),
                is_head: f[4] == "*",
                upstream,
                subject: field(if peeled { 8 } else { 7 }).to_string(),
                committed_at: Some(field(if peeled { 10 } else { 9 }).to_string())
                    .filter(|d| !d.is_empty()),
                ahead,
                behind,
                base_ahead: None,
                base_behind: None,
            })
        })
        .collect()
}

/// Parse `%(upstream:track,nobracket)`: empty when in sync, `ahead 1, behind 2`,
/// or `gone` when the upstream ref no longer exists (both counts then unknown).
fn parse_track(track: &str) -> (Option<u32>, Option<u32>) {
    if track.trim() == "gone" {
        return (None, None);
    }
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(',') {
        let mut words = part.split_whitespace();
        match (words.next(), words.next().and_then(|n| n.parse().ok())) {
            (Some("ahead"), Some(n)) => ahead = n,
            (Some("behind"), Some(n)) => behind = n,
            _ => {}
        }
    }
    (Some(ahead), Some(behind))
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

/// A remote URL without credentials, to store and export as the
/// repository's identity. An `https://token@host/…` or `https://user:pass@…`
/// remote loses its user part; any other `scheme://user:pass@…` keeps the user
/// name but loses the password. An SCP-style `git@host:org/repo` has no
/// password and is kept as it is.
pub fn remote_without_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    // The authority ends at the first `/`; credentials end at its last `@`.
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return url.to_string();
    };
    let web = matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ftp" | "ftps"
    );
    match userinfo.split_once(':') {
        Some((user, _)) if !web => format!("{scheme}://{user}@{host}{path}"),
        None if !web => url.to_string(),
        _ => format!("{scheme}://{host}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_lose_their_credentials() {
        let cases = [
            (
                "https://ghp_secret@github.com/team/web.git",
                "https://github.com/team/web.git",
            ),
            (
                "https://me:pa@ss@example.com/team/web",
                "https://example.com/team/web",
            ),
            (
                "ssh://git:secret@example.com/team/web.git",
                "ssh://git@example.com/team/web.git",
            ),
            (
                "ssh://git@example.com:22/team/web.git",
                "ssh://git@example.com:22/team/web.git",
            ),
            (
                "git@example.com:team/web.git",
                "git@example.com:team/web.git",
            ),
            (
                "https://github.com/team/web.git",
                "https://github.com/team/web.git",
            ),
            ("/Users/someone/code/web", "/Users/someone/code/web"),
        ];
        for (url, kept) in cases {
            assert_eq!(remote_without_credentials(url), kept, "{url}");
        }
    }

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
    fn parses_commit_files_with_renames_and_binaries() {
        let names = b"M\0a.txt\0A\0bin.dat\0R100\0old.txt\0new.txt\0D\0gone.txt\0";
        let stats = b"1\t1\ta.txt\0-\t-\tbin.dat\0" as &[u8];
        let stats = [
            stats,
            b"0\t0\t\0old.txt\0new.txt\0" as &[u8],
            b"0\t3\tgone.txt\0",
        ]
        .concat();
        let files = merge_file_stats(parse_name_status(names), &parse_numstat(&stats));
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].kind, ChangeKind::Modified);
        assert_eq!((files[0].additions, files[0].deletions), (Some(1), Some(1)));
        assert!(files[1].is_binary);
        assert_eq!(files[1].additions, None);
        assert_eq!(files[2].kind, ChangeKind::Renamed);
        assert_eq!(files[2].path, "new.txt");
        assert_eq!(files[2].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[2].additions, Some(0));
        assert!(!files[2].is_binary);
        assert_eq!(
            (files[3].kind, files[3].deletions),
            (ChangeKind::Deleted, Some(3))
        );
    }

    #[test]
    fn parses_commit_header_with_multiline_body() {
        let raw = format!(
            "aaaa{s}aaa{s}bbbb{s}Ann{s}ann@x{s}2026-01-01T00:00:00+00:00{s}2026-01-02T00:00:00+00:00{s}tag: v1{s}Cy{s}cy@x{s}Subject{s}Line one\n\nLine two\n\0",
            s = FIELD_SEP
        );
        let d = parse_commit_header(raw.as_bytes()).unwrap();
        assert_eq!(d.summary.subject, "Subject");
        assert_eq!(d.committer_name, "Cy");
        assert_eq!(d.body, "Line one\n\nLine two");
        assert_eq!(d.summary.parent_ids, vec!["bbbb"]);
    }

    #[test]
    fn chooses_the_compared_parent() {
        let parents = vec!["p1".to_string(), "p2".to_string()];
        assert_eq!(compare_base(&parents, 0).unwrap(), Some("p1"));
        assert_eq!(compare_base(&parents, 1).unwrap(), Some("p2"));
        assert!(compare_base(&parents, 2).is_err());
        assert_eq!(compare_base(&[], 0).unwrap(), None);
        assert!(compare_base(&[], 1).is_err());
    }

    #[test]
    fn parses_refs_and_skips_symbolic_ones() {
        let s = FIELD_SEP;
        let raw = format!(
            "refs/heads/main{s}main{s}c1{s}{s}*{s}origin/main{s}\n\
             refs/remotes/origin/HEAD{s}origin{s}c1{s}{s} {s}{s}refs/remotes/origin/main\n\
             refs/remotes/origin/main{s}origin/main{s}c1{s}{s} {s}{s}\n\
             refs/tags/v1{s}v1{s}t1{s}c0{s} {s}{s}\n"
        );
        let refs = parse_refs(raw.as_bytes());
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].kind, RefKind::LocalBranch);
        assert!(refs[0].is_head);
        assert_eq!(refs[0].upstream.as_deref(), Some("origin/main"));
        assert_eq!(refs[1].kind, RefKind::RemoteBranch);
        assert!(!refs[1].is_head);
        assert_eq!(refs[1].upstream, None);
        assert_eq!(
            (refs[2].kind, refs[2].target_id.as_str()),
            (RefKind::Tag, "c0")
        );
        assert_eq!((refs[0].ahead, refs[0].behind), (Some(0), Some(0)));
        assert_eq!((refs[1].ahead, refs[1].behind), (None, None));
    }

    #[test]
    fn parses_ref_tip_and_tracking() {
        let s = FIELD_SEP;
        let raw = format!(
            "refs/heads/topic{s}topic{s}c2{s}{s} {s}origin/topic{s}{s}Add x{s}{s}2026-01-02T03:04:05+00:00{s}{s}ahead 2, behind 1\n\
             refs/heads/old{s}old{s}c3{s}{s} {s}origin/old{s}{s}Old{s}{s}2026-01-01T00:00:00+00:00{s}{s}gone\n\
             refs/tags/v2{s}v2{s}t2{s}c4{s} {s}{s}{s}Release notes{s}Tip subject{s}2026-02-01T00:00:00+00:00{s}2026-01-31T00:00:00+00:00{s}\n"
        );
        let refs = parse_refs(raw.as_bytes());
        assert_eq!(refs[0].subject, "Add x");
        assert_eq!(
            refs[0].committed_at.as_deref(),
            Some("2026-01-02T03:04:05+00:00")
        );
        assert_eq!((refs[0].ahead, refs[0].behind), (Some(2), Some(1)));
        assert_eq!((refs[1].ahead, refs[1].behind), (None, None));
        // Annotated tags report the tagged commit, not the tag object.
        assert_eq!(refs[2].subject, "Tip subject");
        assert_eq!(
            refs[2].committed_at.as_deref(),
            Some("2026-01-31T00:00:00+00:00")
        );
    }

    #[test]
    fn ref_names_use_the_real_remote_names() {
        let remotes = vec!["origin".to_string(), "team/upstream".to_string()];
        assert_eq!(
            RefName::parse("refs/remotes/team/upstream/main", &remotes),
            RefName::RemoteBranch {
                remote: "team/upstream",
                branch: "main"
            }
        );
        assert_eq!(
            RefName::parse("refs/remotes/origin/release/1.0", &remotes),
            RefName::RemoteBranch {
                remote: "origin",
                branch: "release/1.0"
            }
        );
        assert_eq!(
            RefName::parse("refs/remotes/origin/HEAD", &remotes),
            RefName::Other
        );
        assert_eq!(RefName::parse("refs/tags/v1", &remotes), RefName::Tag("v1"));
    }

    #[test]
    fn refspecs_must_stay_in_remote_tracking_refs() {
        assert!(refspec_is_safe(
            "+refs/heads/*:refs/remotes/origin/*",
            "origin"
        ));
        assert!(refspec_is_safe("refs/tags/*:refs/tags/*", "origin"));
        assert!(!refspec_is_safe("+refs/tags/*:refs/tags/*", "origin"));
        assert!(!refspec_is_safe("+refs/heads/*:refs/tags/*", "origin"));
        assert!(refspec_is_safe("^refs/heads/wip/*", "origin"));
        assert!(!refspec_is_safe("+refs/heads/*:refs/heads/*", "origin"));
        assert!(!refspec_is_safe(
            "+refs/heads/*:refs/remotes/other/*",
            "origin"
        ));
        assert!(!refspec_is_safe("refs/heads/main", "origin"));
        assert!(!refspec_is_safe("--upload-pack=x", "origin"));
    }

    #[test]
    fn finds_the_missing_remote_ref() {
        assert_eq!(
            missing_remote_ref("fatal: couldn't find remote ref refs/heads/master"),
            Some("refs/heads/master".into())
        );
        assert_eq!(
            missing_remote_ref("fatal: couldn't find remote ref develop\n"),
            Some("refs/heads/develop".into())
        );
        assert_eq!(missing_remote_ref("fatal: other"), None);
    }

    #[test]
    fn classifies_fetch_errors_by_whole_phrases() {
        let code = |stderr: &str| classify_fetch_error("origin", stderr).code;
        assert_eq!(
            code("fatal: unable to access 'https://h/x/': The requested URL returned error: 403"),
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            code("fatal: unable to access 'https://h/svc-4031.git/': Could not resolve host: h"),
            ErrorCode::Io
        );
        assert_eq!(
            code("error: cannot lock ref 'refs/remotes/origin/main': is at abc but expected def"),
            ErrorCode::Conflict
        );
        assert_eq!(
            code("fatal: Unable to create '/r/.git/packed-refs.lock': File exists."),
            ErrorCode::Conflict
        );
        assert_eq!(
            code("fatal: unable to create temporary file: No space left on device"),
            ErrorCode::Io
        );
        assert_eq!(
            code("fatal: Authentication failed for 'https://h/x/'"),
            ErrorCode::PermissionDenied
        );
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
