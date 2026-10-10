//! A run's start (SPEC.md, New run; docs/architecture.md, Agent runs — v0.5,
//! Artifacts): one commit and its history, read from the user's repository
//! without writing to it, and copied into Brainiac's own bare repository and
//! a bundle the container clones.
//!
//! - **Checks** refuse what a run cannot copy completely: a shallow or
//!   partial clone, an object format other than SHA-1, submodules, and Git
//!   LFS pointers in the chosen tree. Missing objects are found by the copy
//!   itself. Each refusal says what to do instead.
//! - **The copy** never runs in the user's repository. A temporary bare
//!   repository borrows its objects (an `alternates` file), gets one ref at
//!   the resolved commit, and writes a bundle of that ref; the bundle is
//!   then fetched into `agent-runs/repos/<repository-id>.git` under the
//!   run's own ref. Git reads the repository's own configuration but not
//!   the user's (except the folders it trusts, `safe.directory`), runs no
//!   hook, and a branch that moves meanwhile changes nothing: only the
//!   commit ID is used.
//!
//! Every Git call goes through `GitService::run_isolated`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::git::{validate_revision, GitService, IsolatedGit, Output};
use crate::models::{
    AppError, AppResult, CommitFile, DiffContent, DiffLimits, DiffOptions, DiffResult,
    DiffSelector, ErrorCode, RunStartPreview,
};

/// Reading a repository for New run.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// Copying a large history takes a while.
const COPY_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The ref the bundle advertises, the only one.
pub const BUNDLE_REF: &str = "refs/heads/start";
/// The ref a result bundle advertises, the only one.
pub const RESULT_REF: &str = "refs/heads/result";
/// A result bundle holds only the snapshot's new objects.
const MAX_RESULT_BYTES: u64 = 2 << 30;
/// Git LFS pointer files are small text files starting with this line.
const LFS_POINTER: &str = "^version https://git-lfs\\.github\\.com/spec/v1$";
const LFS_POINTER_MAX_BYTES: u64 = 1024;

/// What a run's start became in Brainiac's data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedStart {
    pub commit: String,
    /// `agent-runs/<run-id>/input.bundle`: this commit and its history,
    /// advertising only `BUNDLE_REF`.
    pub bundle: PathBuf,
    /// The run's ref in Brainiac's bare repository: `refs/brainiac/runs/<run-id>/start`.
    pub start_ref: String,
}

/// A refusal with its remedy, before any container starts or credential is read.
fn refused(reason: &str, remedy: &str) -> AppError {
    AppError::validation(format!("{reason} {remedy}"))
}

/// Folders of `agent-runs/` that are not runs.
const RESERVED: &[&str] = &["repos", "git-home", "empty-template", "controller"];

/// IDs become folder and ref names: letters, digits, and dashes only, and
/// never one of Brainiac's own folders.
fn check_id(what: &str, id: &str) -> AppResult<()> {
    if RESERVED.contains(&id)
        || id.is_empty()
        || id.len() > 64
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(AppError::validation(format!("Invalid {what} ID.")));
    }
    Ok(())
}

pub struct RunArtifacts {
    /// `None` when Git is missing: every action then explains that.
    git: Option<GitService>,
    /// `agent-runs/` in the data folder.
    root: PathBuf,
    /// One import into each shared bare repository at a time.
    // `Arc<tokio::sync::Mutex<()>>` per repository: an async lock held across
    // the fetch, shared so the map's own lock is never held while waiting.
    imports: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl RunArtifacts {
    pub fn new(git: Option<GitService>, data_dir: &Path) -> Self {
        RunArtifacts {
            git,
            root: data_dir.join("agent-runs"),
            imports: Mutex::new(HashMap::new()),
        }
    }

    /// Brainiac's own bare repository for a registered repository's runs.
    pub fn repository_dir(&self, repository_id: &str) -> PathBuf {
        self.root.join("repos").join(format!("{repository_id}.git"))
    }

    /// An empty folder Git takes as its home, so it reads no user configuration.
    fn home(&self) -> AppResult<PathBuf> {
        let home = self.root.join("git-home");
        std::fs::create_dir_all(&home).map_err(|e| {
            AppError::io("Brainiac could not create its runs folder.").with_details(e.to_string())
        })?;
        Ok(home)
    }

    async fn git(&self, cwd: &Path, args: &[&str], timeout: Duration) -> AppResult<Output> {
        let git = self.git.as_ref().ok_or_else(|| {
            AppError::dependency("Git was not found, so Brainiac cannot read the repository.")
        })?;
        // The user's own configuration is left out, except the folders they
        // told Git to trust although someone else owns them (an external
        // or shared volume): elsewhere Brainiac reads those repositories too.
        let trusted: Vec<String> = git
            .user_config_values("safe.directory")
            .await
            .into_iter()
            .map(|dir| format!("safe.directory={dir}"))
            .collect();
        let mut all: Vec<&str> = trusted.iter().flat_map(|t| ["-c", t.as_str()]).collect();
        all.extend_from_slice(args);
        git.run_isolated(cwd, &all, &self.home()?, timeout).await
    }

    /// Run Git and return what it printed, or an error naming the command.
    async fn git_ok(&self, cwd: &Path, args: &[&str], timeout: Duration) -> AppResult<String> {
        let out = self.git(cwd, args, timeout).await?;
        if !out.success() {
            return Err(
                AppError::io("Git could not read the repository.").with_details(format!(
                    "git {}\n{}",
                    args.join(" "),
                    out.stderr
                )),
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// New run, **Start from**: check the repository, resolve `start` once
    /// to a commit, and check that commit's tree. Reads only.
    pub async fn preview(
        &self,
        repository_id: &str,
        root: &Path,
        start: &str,
    ) -> AppResult<RunStartPreview> {
        let start = start.trim();
        validate_revision(start)?;
        self.check_repository(root).await?;
        let commit = self.resolve(root, start).await?;
        self.check_tree(root, &commit).await?;
        let count = self
            .git_ok(root, &["rev-list", "--count", &commit, "--"], CHECK_TIMEOUT)
            .await?;
        let header = self
            .git_ok(
                root,
                &[
                    "log",
                    "-1",
                    "--no-show-signature",
                    "--format=%s%x1f%an%x1f%cI",
                    &commit,
                    "--",
                ],
                CHECK_TIMEOUT,
            )
            .await?;
        let mut fields = header.trim_end_matches('\n').split('\u{1f}');
        Ok(RunStartPreview {
            repository_id: repository_id.to_string(),
            subject: fields.next().unwrap_or_default().to_string(),
            author: fields.next().unwrap_or_default().to_string(),
            committed_at: fields.next().unwrap_or_default().to_string(),
            history_commits: count.trim().parse().unwrap_or(0),
            commit,
        })
    }

    /// A complete SHA-1 repository: not shallow, not a partial clone.
    async fn check_repository(&self, root: &Path) -> AppResult<()> {
        let out = self
            .git_ok(
                root,
                &[
                    "rev-parse",
                    "--is-shallow-repository",
                    "--show-object-format",
                ],
                CHECK_TIMEOUT,
            )
            .await
            .map_err(|e| {
                AppError::validation("This folder is not a Git repository Brainiac can read.")
                    .with_details(e.details.unwrap_or_default())
            })?;
        let mut lines = out.lines();
        if lines.next() == Some("true") {
            return Err(refused(
                "This repository is a shallow clone, so a run cannot get the commit's whole history.",
                "Fetch the rest with git fetch --unshallow, then try again.",
            ));
        }
        let format = lines.next().unwrap_or_default();
        if format != "sha1" {
            return Err(refused(
                &format!("This repository uses {format} object IDs."),
                "Runs support SHA-1 repositories only.",
            ));
        }
        // Exit status 1: no such setting, a complete clone.
        let partial = self
            .git(
                root,
                &[
                    "config",
                    "--get-regexp",
                    "^(extensions\\.partialclone|remote\\..*\\.promisor|remote\\..*\\.partialclonefilter)$",
                ],
                CHECK_TIMEOUT,
            )
            .await?;
        let promised = String::from_utf8_lossy(&partial.stdout)
            .lines()
            .any(|l| !l.ends_with(" false"));
        if partial.success() && promised {
            return Err(refused(
                "This repository is a partial clone: some objects were never downloaded.",
                "Clone it again without --filter, then try again.",
            ));
        }
        Ok(())
    }

    /// `start` as a commit ID, resolved once.
    async fn resolve(&self, root: &Path, start: &str) -> AppResult<String> {
        let spec = format!("{start}^{{commit}}");
        let out = self
            .git(
                root,
                &["rev-parse", "--verify", "--quiet", &spec],
                CHECK_TIMEOUT,
            )
            .await?;
        let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.success() || commit.len() != 40 {
            return Err(AppError::not_found(format!(
                "There is no branch or commit {start} in this repository."
            )));
        }
        Ok(commit)
    }

    /// No submodules and no Git LFS pointers in the commit's tree.
    async fn check_tree(&self, root: &Path, commit: &str) -> AppResult<()> {
        let out = self
            .git(
                root,
                &["ls-tree", "-r", "-z", "-l", "--full-tree", commit],
                CHECK_TIMEOUT,
            )
            .await?;
        // A cut listing ends Git early, so it is checked before the exit status.
        if out.truncated {
            return Err(AppError::validation(
                "This commit has too many files for Brainiac to check before a run.",
            ));
        }
        if !out.success() {
            return Err(missing_objects(&out.stderr));
        }
        let mut lfs = false;
        let mut small: HashMap<String, u64> = HashMap::new();
        for entry in out.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let entry = String::from_utf8_lossy(entry);
            let Some((meta, path)) = entry.split_once('\t') else {
                continue;
            };
            let mut parts = meta.split_whitespace();
            let (mode, kind, id, size) = (parts.next(), parts.next(), parts.next(), parts.next());
            if mode == Some("160000") || kind == Some("commit") {
                return Err(refused(
                    &format!("This commit has a submodule at {path}."),
                    "Runs cannot include submodules yet; start from a commit without them.",
                ));
            }
            if path == ".gitattributes" || path.ends_with("/.gitattributes") {
                if let Some(id) = id {
                    let text = self
                        .git_ok(root, &["cat-file", "blob", id], CHECK_TIMEOUT)
                        .await?;
                    lfs |= text.contains("filter=lfs");
                }
            }
            if let Some(size) = size.and_then(|s| s.parse::<u64>().ok()) {
                if size <= LFS_POINTER_MAX_BYTES {
                    small.insert(path.to_string(), size);
                }
            }
        }
        if !lfs {
            return Ok(());
        }
        // Exit status 1: no file matched.
        let out = self
            .git(
                root,
                &["grep", "-l", "-z", "-E", "-e", LFS_POINTER, commit, "--"],
                // Every blob of the commit is searched.
                COPY_TIMEOUT,
            )
            .await?;
        let prefix = format!("{commit}:");
        let pointer = out
            .stdout
            .split(|b| *b == 0)
            .map(String::from_utf8_lossy)
            .filter_map(|p| p.strip_prefix(&prefix).map(str::to_string))
            .find(|p| small.contains_key(p));
        if let Some(path) = pointer {
            return Err(refused(
                &format!("{path} is a Git LFS pointer, not the file itself."),
                "Runs cannot fetch Git LFS files yet; start from a commit without them.",
            ));
        }
        Ok(())
    }

    /// Copy `commit` and its history from the repository at `root` into
    /// Brainiac's bare repository for `repository_id`, under the run's ref,
    /// and write the run's input bundle. Writes nothing in `root`.
    pub async fn export(
        &self,
        repository_id: &str,
        root: &Path,
        commit: &str,
        run_id: &str,
    ) -> AppResult<ExportedStart> {
        check_id("repository", repository_id)?;
        check_id("run", run_id)?;
        if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AppError::validation("Invalid commit ID."));
        }
        let objects = self.objects_dir(root).await?;
        let run_dir = self.root.join(run_id);
        let bundle = run_dir.join("input.bundle");
        let export = run_dir.join("export.git");
        let io = |e: std::io::Error| {
            AppError::io("Brainiac could not write the run's files.").with_details(e.to_string())
        };
        std::fs::create_dir_all(&run_dir).map_err(io)?;
        if export.exists() {
            std::fs::remove_dir_all(&export).map_err(io)?;
        }
        let copied = async {
            self.init_bare(&run_dir, &export).await?;
            // The temporary repository reads the user's objects in place.
            std::fs::write(
                export.join("objects/info/alternates"),
                format!("{}\n", objects.display()),
            )
            .map_err(io)?;
            self.git_ok(&export, &["update-ref", BUNDLE_REF, commit], CHECK_TIMEOUT)
                .await
                .map_err(|e| missing_objects(e.details.as_deref().unwrap_or_default()))?;
            let bundle_text = bundle.display().to_string();
            let out = self
                .git(
                    &export,
                    &["bundle", "create", &bundle_text, BUNDLE_REF],
                    COPY_TIMEOUT,
                )
                .await?;
            if !out.success() {
                return Err(missing_objects(&out.stderr));
            }
            Ok(())
        }
        .await;
        // The temporary repository holds no objects of its own.
        let _ = std::fs::remove_dir_all(&export);
        let start_ref = format!("refs/brainiac/runs/{run_id}/start");
        let kept = match copied {
            Ok(()) => match self.verify_bundle(&run_dir, &bundle, commit).await {
                Ok(()) => {
                    self.import(repository_id, &bundle, &start_ref, commit)
                        .await
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        if let Err(e) = kept {
            // Nothing of a failed start is left behind: no bundle, and the
            // run's folder too when nothing else is in it.
            let _ = std::fs::remove_file(&bundle);
            let _ = std::fs::remove_dir(&run_dir);
            return Err(e);
        }
        Ok(ExportedStart {
            commit: commit.to_string(),
            bundle,
            start_ref,
        })
    }

    /// The user's repository's object folder, as an absolute path.
    async fn objects_dir(&self, root: &Path) -> AppResult<PathBuf> {
        let out = self
            .git_ok(root, &["rev-parse", "--git-common-dir"], CHECK_TIMEOUT)
            .await?;
        let common = Path::new(out.trim());
        let common = if common.is_absolute() {
            common.to_path_buf()
        } else {
            root.join(common)
        };
        std::fs::canonicalize(common.join("objects")).map_err(|e| {
            AppError::io("Brainiac could not find the repository's objects.")
                .with_details(e.to_string())
        })
    }

    /// A new bare repository with no hooks or other template files.
    async fn init_bare(&self, cwd: &Path, path: &Path) -> AppResult<()> {
        let template = self.root.join("empty-template");
        std::fs::create_dir_all(&template).map_err(|e| {
            AppError::io("Brainiac could not create its runs folder.").with_details(e.to_string())
        })?;
        let (template, path) = (
            format!("--template={}", template.display()),
            path.display().to_string(),
        );
        self.git_ok(
            cwd,
            &[
                "-c",
                "init.defaultBranch=start",
                "init",
                "--quiet",
                "--bare",
                &template,
                &path,
            ],
            CHECK_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    /// The bundle advertises exactly one ref, at exactly the commit.
    async fn verify_bundle(&self, cwd: &Path, bundle: &Path, commit: &str) -> AppResult<()> {
        let path = bundle.display().to_string();
        let heads = self
            .git_ok(cwd, &["bundle", "list-heads", &path], CHECK_TIMEOUT)
            .await?;
        let expected = format!("{commit} {BUNDLE_REF}");
        if heads.lines().collect::<Vec<_>>() != [expected.as_str()] {
            return Err(AppError::io(
                "The run's copy of the repository is not what was asked for.",
            )
            .with_details(heads));
        }
        Ok(())
    }

    /// Fetch the bundle into Brainiac's bare repository, one import into it
    /// at a time, and check the run's ref points at the commit.
    async fn import(
        &self,
        repository_id: &str,
        bundle: &Path,
        start_ref: &str,
        commit: &str,
    ) -> AppResult<()> {
        let lock = Arc::clone(
            self.imports
                .lock()
                .expect("imports lock")
                .entry(repository_id.to_string())
                .or_default(),
        );
        let _held = lock.lock().await;
        let repo = self.repository_dir(repository_id);
        if !repo.join("HEAD").exists() {
            let parent = repo.parent().expect("repos folder");
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::io("Brainiac could not create its runs folder.")
                    .with_details(e.to_string())
            })?;
            self.init_bare(parent, &repo).await?;
        }
        let (source, refspec) = (
            bundle.display().to_string(),
            format!("{BUNDLE_REF}:{start_ref}"),
        );
        self.git_ok(
            &repo,
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                &source,
                &refspec,
            ],
            COPY_TIMEOUT,
        )
        .await?;
        let found = self
            .git_ok(
                &repo,
                &["rev-parse", "--verify", "--quiet", start_ref],
                CHECK_TIMEOUT,
            )
            .await?;
        if found.trim() != commit {
            return Err(AppError::io("The run's start was not kept as copied."));
        }
        Ok(())
    }
}

impl RunArtifacts {
    /// `agent-runs/<run-id>/`: the run's bundle, mirrored journal, and result.
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.root.join(run_id)
    }

    /// A run's ref in Brainiac's bare repository.
    pub fn run_ref(run_id: &str, name: &str) -> String {
        format!("refs/brainiac/runs/{run_id}/{name}")
    }

    /// Import a collected `result.bundle` (docs/architecture.md, Agent runs —
    /// v0.5, Artifacts): one regular file of bounded size, one advertised
    /// ref at exactly `result`, whose only parent is `start`, fetched with
    /// Git's own object checks into the repository's bare repository under
    /// the run's result ref. Returns the result commit.
    pub async fn import_result(
        &self,
        repository_id: &str,
        run_id: &str,
        bundle: &Path,
        start: &str,
        result: &str,
    ) -> AppResult<String> {
        self.import_snapshot(repository_id, run_id, bundle, start, result, false)
            .await
    }

    /// A live run's previews (Changes so far): `agent-runs/<run-id>/preview.git`,
    /// a repository of the run's own that borrows the start from Brainiac's
    /// repository (an `alternates` file) and holds only the latest preview.
    /// It never stands for the result, and it goes with the run's folder.
    pub fn preview_repository(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("preview.git")
    }

    /// The same checks for a live run's provisional snapshot, imported into
    /// a fresh preview repository that replaces the last one, so previews
    /// never pile up in Brainiac's repository.
    pub async fn import_preview(
        &self,
        repository_id: &str,
        run_id: &str,
        bundle: &Path,
        start: &str,
        result: &str,
    ) -> AppResult<String> {
        self.import_snapshot(repository_id, run_id, bundle, start, result, true)
            .await
    }

    async fn import_snapshot(
        &self,
        repository_id: &str,
        run_id: &str,
        bundle: &Path,
        start: &str,
        result: &str,
        preview: bool,
    ) -> AppResult<String> {
        check_id("repository", repository_id)?;
        check_id("run", run_id)?;
        for id in [start, result] {
            if id.len() != 40 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(AppError::validation("Invalid commit ID."));
            }
        }
        let meta = std::fs::symlink_metadata(bundle).map_err(|e| {
            AppError::io("The collected result is missing.").with_details(e.to_string())
        })?;
        if !meta.is_file() {
            return Err(AppError::io("The collected result is not a regular file."));
        }
        if meta.len() > MAX_RESULT_BYTES {
            return Err(AppError::validation(
                "The collected result is over the 2 GB limit.",
            ));
        }
        let repo = self.repository_dir(repository_id);
        let start_ref = Self::run_ref(run_id, "start");
        let result_ref = Self::run_ref(run_id, if preview { "preview" } else { "result" });
        let lock = Arc::clone(
            self.imports
                .lock()
                .expect("imports lock")
                .entry(repository_id.to_string())
                .or_default(),
        );
        let _held = lock.lock().await;
        let known = self
            .git_ok(
                &repo,
                &["rev-parse", "--verify", "--quiet", &start_ref],
                CHECK_TIMEOUT,
            )
            .await?;
        if known.trim() != start {
            return Err(AppError::io(
                "The run's start is no longer in Brainiac's repository.",
            ));
        }
        // A preview goes into a repository made for it next to the last one,
        // which it replaces once it holds the preview.
        let (repo, made) = if preview {
            let made = self.run_dir(run_id).join("preview-next.git");
            let io = |e: std::io::Error| {
                AppError::io("The preview's repository could not be made.")
                    .with_details(e.to_string())
            };
            if made.exists() {
                std::fs::remove_dir_all(&made).map_err(io)?;
            }
            self.init_bare(&self.run_dir(run_id), &made).await?;
            let objects = std::fs::canonicalize(repo.join("objects")).map_err(io)?;
            std::fs::write(
                made.join("objects/info/alternates"),
                format!("{}\n", objects.display()),
            )
            .map_err(io)?;
            (made.clone(), Some(made))
        } else {
            (repo, None)
        };
        let path = bundle.display().to_string();
        let heads = self
            .git_ok(&repo, &["bundle", "list-heads", &path], CHECK_TIMEOUT)
            .await
            .map_err(|e| {
                AppError::io("The collected result is not a Git bundle.")
                    .with_details(e.details.unwrap_or_default())
            })?;
        if heads.lines().collect::<Vec<_>>() != [format!("{result} {RESULT_REF}").as_str()] {
            return Err(AppError::io(
                "The collected result does not advertise the snapshot it says it holds.",
            )
            .with_details(heads));
        }
        // `verify` checks that every prerequisite (the start) is here.
        self.git_ok(
            &repo,
            &["bundle", "verify", "--quiet", &path],
            CHECK_TIMEOUT,
        )
        .await
        .map_err(|e| {
            AppError::io("The collected result does not fit the run's start.")
                .with_details(e.details.unwrap_or_default())
        })?;
        let refspec = format!("+{RESULT_REF}:{result_ref}");
        self.git_ok(
            &repo,
            &[
                "-c",
                "fetch.fsckObjects=true",
                "-c",
                "transfer.fsckObjects=true",
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                &path,
                &refspec,
            ],
            COPY_TIMEOUT,
        )
        .await?;
        let parents = self
            .git_ok(
                &repo,
                &["rev-list", "--parents", "-n", "1", &result_ref, "--"],
                CHECK_TIMEOUT,
            )
            .await?;
        let ids: Vec<&str> = parents.split_whitespace().collect();
        if ids != [result, start] {
            // Not what was promised: the ref is not left pointing at it.
            let _ = self
                .git(&repo, &["update-ref", "-d", &result_ref], CHECK_TIMEOUT)
                .await;
            return Err(AppError::io(
                "The collected snapshot is not one commit on top of the run's start.",
            ));
        }
        if let Some(made) = made {
            let current = self.preview_repository(run_id);
            let io = |e: std::io::Error| {
                AppError::io("The preview's repository could not be replaced.")
                    .with_details(e.to_string())
            };
            if current.exists() {
                std::fs::remove_dir_all(&current).map_err(io)?;
            }
            std::fs::rename(&made, &current).map_err(io)?;
        }
        Ok(result.to_string())
    }

    /// Where a snapshot is: Brainiac's repository, or the run's preview
    /// repository for Changes so far.
    fn snapshot_dir(&self, repository_id: &str, preview_of: Option<&str>) -> PathBuf {
        match preview_of {
            Some(run_id) => self.preview_repository(run_id),
            None => self.repository_dir(repository_id),
        }
    }

    /// Changes: the files the snapshot changed against the start;
    /// `preview_of` names the run whose preview it is.
    pub async fn changes(
        &self,
        repository_id: &str,
        start: &str,
        result: &str,
        preview_of: Option<&str>,
    ) -> AppResult<Vec<CommitFile>> {
        let git = self.git.as_ref().ok_or_else(|| {
            AppError::dependency("Git was not found, so Brainiac cannot read the result.")
        })?;
        git.range_files(&self.snapshot_dir(repository_id, preview_of), start, result)
            .await
    }

    /// One file of the snapshot against the start, as the diff viewer shows
    /// it, between the start and the snapshot.
    pub async fn diff(
        &self,
        repository_id: &str,
        (start, result): (&str, &str),
        path: &str,
        old_path: Option<&str>,
        options: DiffOptions,
        preview_of: Option<&str>,
    ) -> AppResult<DiffResult> {
        let git = self.git.as_ref().ok_or_else(|| {
            AppError::dependency("Git was not found, so Brainiac cannot read the result.")
        })?;
        let selector = DiffSelector::Range {
            base: start.to_string(),
            head: result.to_string(),
            path: path.to_string(),
            old_path: old_path.map(str::to_string),
        };
        let limits = DiffLimits {
            max_bytes: 4 * 1024 * 1024,
            max_lines: 20_000,
        };
        let content: DiffContent = git
            .diff(
                &self.snapshot_dir(repository_id, preview_of),
                &selector,
                &limits,
                options,
            )
            .await?;
        Ok(DiffResult {
            repository_id: repository_id.to_string(),
            selector,
            content,
        })
    }

    /// The whole snapshot as a patch, binary changes included: to the
    /// clipboard when it fits in `MAX_OUTPUT_BYTES`, or straight to a file.
    pub async fn patch(
        &self,
        repository_id: &str,
        start: &str,
        result: &str,
        to_file: Option<&Path>,
    ) -> AppResult<String> {
        let repo = self.repository_dir(repository_id);
        let output = to_file.map(|p| format!("--output={}", p.display()));
        let mut args = vec![
            "diff",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
        ];
        if let Some(output) = &output {
            args.push(output);
        }
        args.extend(["--end-of-options", start, result, "--"]);
        let out = self.git(&repo, &args, COPY_TIMEOUT).await?;
        if !out.success() {
            return Err(AppError::io("The patch could not be made.").with_details(out.stderr));
        }
        if out.truncated {
            return Err(AppError::validation(
                "The patch is over 16 MB, too large to copy. Save it to a file instead.",
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// **Copy branch command** (SPEC.md, Review): a Git command the user runs
    /// in their own repository, which fetches the snapshot from Brainiac's
    /// repository as a new branch. Brainiac never runs it: the user's
    /// repository is only read. Without `+`, Git refuses to move a branch
    /// that holds other work; it only moves one that is behind the snapshot.
    pub fn branch_command(&self, repository_id: &str, run_id: &str, branch: &str) -> String {
        let repo = self.repository_dir(repository_id);
        let refspec = format!("{}:refs/heads/{branch}", Self::run_ref(run_id, "result"));
        format!(
            "git fetch --no-tags {} {}",
            shell_quote(&repo.display().to_string()),
            shell_quote(&refspec)
        )
    }

    /// Settings → Agents, **Test**: a tiny repository made at `source`, its
    /// one commit exported for the test's run. Returns the bundle and the commit.
    pub async fn test_bundle(&self, source: &Path, run_id: &str) -> AppResult<(PathBuf, String)> {
        let io = |e: std::io::Error| {
            AppError::io("The test's repository could not be made.").with_details(e.to_string())
        };
        if source.exists() {
            std::fs::remove_dir_all(source).map_err(io)?;
        }
        std::fs::create_dir_all(source).map_err(io)?;
        std::fs::write(
            source.join("README.md"),
            "# Brainiac test run\n\nA repository made for Settings → Agents, Test.\n",
        )
        .map_err(io)?;
        let identity = [
            "-c",
            "user.name=Brainiac",
            "-c",
            "user.email=runs@brainiac.invalid",
        ];
        let mut init = identity.to_vec();
        init.extend(["-c", "init.defaultBranch=main", "init", "--quiet"]);
        self.git_ok(source, &init, CHECK_TIMEOUT).await?;
        self.git_ok(source, &["add", "README.md"], CHECK_TIMEOUT)
            .await?;
        let mut commit = identity.to_vec();
        commit.extend(["commit", "--quiet", "-m", "Test"]);
        self.git_ok(source, &commit, CHECK_TIMEOUT).await?;
        let head = self
            .git_ok(source, &["rev-parse", "HEAD"], CHECK_TIMEOUT)
            .await?
            .trim()
            .to_string();
        let exported = self.export("test", source, &head, run_id).await?;
        Ok((exported.bundle, head))
    }

    /// Delete a run's files and its refs in Brainiac's repository. Objects
    /// other runs share stay; unreachable ones go with Git's own upkeep.
    pub async fn remove_run(&self, repository_id: &str, run_id: &str) -> AppResult<()> {
        check_id("run", run_id)?;
        let dir = self.run_dir(run_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                AppError::io("The run's files could not be removed.").with_details(e.to_string())
            })?;
        }
        if check_id("repository", repository_id).is_ok() {
            let repo = self.repository_dir(repository_id);
            if repo.join("HEAD").exists() {
                for name in ["start", "result"] {
                    let r = Self::run_ref(run_id, name);
                    let _ = self
                        .git(&repo, &["update-ref", "-d", &r], CHECK_TIMEOUT)
                        .await;
                }
            }
        }
        Ok(())
    }
}

impl RunArtifacts {
    /// An explain run's files once its explanation is read: its input
    /// bundle, everything else in its folder but the mirrored journal
    /// (`trace.jsonl`, kept for How it was written), and its refs in
    /// Brainiac's repository.
    pub async fn remove_run_keeping_trace(
        &self,
        repository_id: &str,
        run_id: &str,
    ) -> AppResult<()> {
        check_id("run", run_id)?;
        let dir = self.run_dir(run_id);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.file_name() == "trace.jsonl" {
                    continue;
                }
                let path = entry.path();
                let removed = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                removed.map_err(|e| {
                    AppError::io("The run's files could not be removed.")
                        .with_details(e.to_string())
                })?;
            }
        }
        if check_id("repository", repository_id).is_ok() {
            let repo = self.repository_dir(repository_id);
            if repo.join("HEAD").exists() {
                for name in ["start", "result"] {
                    let r = Self::run_ref(run_id, name);
                    let _ = self
                        .git(&repo, &["update-ref", "-d", &r], CHECK_TIMEOUT)
                        .await;
                }
            }
        }
        Ok(())
    }

    /// Git for reading Brainiac's own copy, or the user's repository, with
    /// none of the user's configuration, in this folder's empty home and with
    /// the timeout of a check (`explain::subject` reads changes with it).
    pub fn isolated_git(&self) -> AppResult<IsolatedGit> {
        let git = self.git.clone().ok_or_else(|| {
            AppError::dependency("Git was not found, so Brainiac cannot read the change.")
        })?;
        Ok(IsolatedGit::new(git, self.home()?, CHECK_TIMEOUT))
    }

    /// Run a reading Git command in `repo` and return what it printed.
    pub async fn read_git(&self, repo: &Path, args: &[&str]) -> AppResult<String> {
        self.git_ok(repo, args, CHECK_TIMEOUT).await
    }
}

/// One word for a POSIX shell: single quotes, a quote inside closed and
/// escaped. The command is pasted into the user's terminal.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A branch name for a run's snapshot: `agent/` and a few words of its
/// title, then the start of its ID, so two runs never share one. Only
/// lowercase letters, digits, and dashes, which Git accepts in a ref.
pub fn branch_name(title: &str, run_id: &str) -> String {
    let mut words = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            words.push(c);
        } else if !words.ends_with('-') && !words.is_empty() {
            words.push('-');
        }
        if words.len() >= 32 {
            break;
        }
    }
    let words = words.trim_matches('-');
    let id: String = run_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>()
        .to_lowercase();
    if words.is_empty() {
        format!("agent/run-{id}")
    } else {
        format!("agent/{words}-{id}")
    }
}

/// Git could not read an object the commit needs.
fn missing_objects(stderr: &str) -> AppError {
    AppError::new(
        ErrorCode::Validation,
        "Part of this commit's history is missing from the repository, so a run cannot copy it completely. Run git fsck to find what is missing, fetch it again, then try again.",
    )
    .with_details(stderr.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "protocol.file.allow=always",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repository with two commits on `main`, written by the test.
    fn repo(base: &Path) -> PathBuf {
        let dir = base.join("source");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "--quiet"]);
        std::fs::write(dir.join("readme.txt"), "one\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "--quiet", "-m", "First"]);
        std::fs::write(dir.join("readme.txt"), "two\n").unwrap();
        git(&dir, &["commit", "--quiet", "-am", "Second"]);
        dir
    }

    async fn artifacts(base: &Path) -> RunArtifacts {
        let git = GitService::detect().await.unwrap();
        RunArtifacts::new(Some(git), &base.join("data"))
    }

    /// Everything Git keeps about the repository besides its objects.
    fn state(dir: &Path) -> Vec<String> {
        vec![
            git(dir, &["for-each-ref"]),
            git(dir, &["status", "--porcelain=v2", "--branch"]),
            std::fs::read_to_string(dir.join(".git/config")).unwrap(),
            git(dir, &["stash", "list"]),
        ]
    }

    #[tokio::test]
    async fn a_branch_resolves_once_to_its_commit_and_history() {
        let base = tempfile::tempdir().unwrap();
        let source = repo(base.path());
        let artifacts = artifacts(base.path()).await;
        let preview = artifacts.preview("repo-1", &source, "main").await.unwrap();
        assert_eq!(preview.commit, git(&source, &["rev-parse", "main"]));
        assert_eq!(preview.subject, "Second");
        assert_eq!(preview.author, "Test");
        assert_eq!(preview.history_commits, 2);
        let err = artifacts
            .preview("repo-1", &source, "nowhere")
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(artifacts.preview("repo-1", &source, "--all").await.is_err());
    }

    #[tokio::test]
    async fn the_export_copies_one_commit_and_writes_nothing_in_the_repository() {
        let base = tempfile::tempdir().unwrap();
        let source = repo(base.path());
        std::fs::write(source.join("readme.txt"), "uncommitted\n").unwrap();
        git(&source, &["branch", "other"]);
        let artifacts = artifacts(base.path()).await;
        let commit = git(&source, &["rev-parse", "main"]);
        let before = state(&source);

        // The branch moves after the start was resolved: the run keeps its commit.
        std::fs::write(source.join("new.txt"), "later\n").unwrap();
        git(&source, &["add", "new.txt"]);
        git(&source, &["commit", "--quiet", "-m", "Third"]);
        let before_export = state(&source);

        let start = artifacts
            .export("repo-1", &source, &commit, "run-1")
            .await
            .unwrap();
        assert_eq!(state(&source), before_export);
        assert_ne!(before, before_export);
        assert_eq!(start.start_ref, "refs/brainiac/runs/run-1/start");

        let heads = git(
            base.path(),
            &["bundle", "list-heads", start.bundle.to_str().unwrap()],
        );
        assert_eq!(heads, format!("{commit} {BUNDLE_REF}"));
        let repo = artifacts.repository_dir("repo-1");
        assert_eq!(git(&repo, &["rev-parse", &start.start_ref]), commit);
        assert_eq!(git(&repo, &["rev-list", "--count", &start.start_ref]), "2");
        // Brainiac's repository has its own objects, not borrowed ones, and no hooks.
        assert!(!repo.join("objects/info/alternates").exists());
        assert!(std::fs::read_dir(repo.join("hooks")).map_or(true, |mut d| d.next().is_none()));
        assert!(!base
            .path()
            .join("data/agent-runs/run-1/export.git")
            .exists());

        // The container clones the bundle on its own.
        let clone = base.path().join("clone");
        git(
            base.path(),
            &[
                "clone",
                "--quiet",
                "--branch",
                "start",
                start.bundle.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        assert_eq!(
            std::fs::read_to_string(clone.join("readme.txt")).unwrap(),
            "two\n"
        );
        assert!(!clone.join("new.txt").exists());

        // A second run of the same repository shares Brainiac's repository.
        let second = artifacts
            .export("repo-1", &source, &commit, "run-2")
            .await
            .unwrap();
        assert_eq!(git(&repo, &["rev-parse", &second.start_ref]), commit);
    }

    #[tokio::test]
    async fn incomplete_repositories_are_refused_with_a_remedy() {
        let base = tempfile::tempdir().unwrap();
        let source = repo(base.path());
        let artifacts = artifacts(base.path()).await;

        let shallow = base.path().join("shallow");
        let url = format!("file://{}", source.display());
        git(
            base.path(),
            &[
                "clone",
                "--quiet",
                "--depth",
                "1",
                &url,
                shallow.to_str().unwrap(),
            ],
        );
        let err = artifacts.preview("r", &shallow, "HEAD").await.unwrap_err();
        assert!(
            err.message.contains("shallow") && err.message.contains("--unshallow"),
            "{err:?}"
        );

        let partial = base.path().join("partial");
        git(
            base.path(),
            &[
                "clone",
                "--quiet",
                "--no-local",
                &url,
                partial.to_str().unwrap(),
            ],
        );
        git(&partial, &["config", "remote.origin.promisor", "true"]);
        let err = artifacts.preview("r", &partial, "HEAD").await.unwrap_err();
        assert!(err.message.contains("partial clone"), "{err:?}");
    }

    #[tokio::test]
    async fn submodules_and_lfs_pointers_are_refused() {
        let base = tempfile::tempdir().unwrap();
        let source = repo(base.path());
        let artifacts = artifacts(base.path()).await;
        let head = git(&source, &["rev-parse", "HEAD"]);

        git(
            &source,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{head},vendor/lib"),
            ],
        );
        git(&source, &["commit", "--quiet", "-m", "Submodule"]);
        let err = artifacts.preview("r", &source, "HEAD").await.unwrap_err();
        assert!(err.message.contains("submodule at vendor/lib"), "{err:?}");
        git(&source, &["reset", "--quiet", "--hard", &head]);

        std::fs::write(
            source.join(".gitattributes"),
            "*.bin filter=lfs diff=lfs merge=lfs -text\n",
        )
        .unwrap();
        let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
        std::fs::write(source.join("model.bin"), pointer).unwrap();
        // Written as plain blobs: the test has no LFS filter.
        git(&source, &["-c", "filter.lfs.clean=cat", "add", "."]);
        git(&source, &["commit", "--quiet", "-m", "LFS"]);
        let err = artifacts.preview("r", &source, "HEAD").await.unwrap_err();
        assert!(
            err.message.contains("model.bin is a Git LFS pointer"),
            "{err:?}"
        );
        // Attributes outside the commit that call it binary change nothing.
        std::fs::write(source.join(".git/info/attributes"), "*.bin binary\n").unwrap();
        let err = artifacts.preview("r", &source, "HEAD").await.unwrap_err();
        assert!(err.message.contains("model.bin"), "{err:?}");

        // LFS attributes alone, with no pointer file, are fine.
        git(&source, &["rm", "--quiet", "model.bin"]);
        git(&source, &["commit", "--quiet", "-m", "No pointer"]);
        artifacts.preview("r", &source, "HEAD").await.unwrap();
    }

    #[tokio::test]
    async fn missing_objects_fail_the_copy_before_anything_is_kept() {
        let base = tempfile::tempdir().unwrap();
        let source = repo(base.path());
        let artifacts = artifacts(base.path()).await;
        let commit = git(&source, &["rev-parse", "HEAD"]);
        // The first commit's file, a loose object, disappears.
        let blob = git(&source, &["rev-parse", "HEAD~1:readme.txt"]);
        std::fs::remove_file(
            source
                .join(".git/objects")
                .join(&blob[..2])
                .join(&blob[2..]),
        )
        .unwrap();

        let err = artifacts
            .export("repo-1", &source, &commit, "run-1")
            .await
            .unwrap_err();
        assert!(err.message.contains("missing"), "{err:?}");
        assert!(!artifacts.repository_dir("repo-1").exists());
        assert!(!base.path().join("data/agent-runs/run-1").exists());
        assert!(artifacts
            .export("../x", &source, &commit, "run-1")
            .await
            .is_err());
        assert!(artifacts
            .export("r", &source, &commit, "empty-template")
            .await
            .is_err());
    }

    #[test]
    fn a_branch_name_is_a_few_words_of_the_title_and_the_run() {
        assert_eq!(
            branch_name("Fix the login page's focus ring!", "B2FAB68C-20a0-42db"),
            "agent/fix-the-login-page-s-focus-ring-b2fab68c"
        );
        assert_eq!(branch_name("¿?", "run-1"), "agent/run-run1");
        let long = branch_name(&"word ".repeat(40), "abc");
        assert!(long.len() < 50, "{long}");
        assert!(!long.contains("--"));
    }

    #[test]
    fn a_quoted_path_survives_spaces_and_quotes() {
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\\''s'");
    }
}
