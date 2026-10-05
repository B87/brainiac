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

use crate::git::{validate_revision, GitService, Output};
use crate::models::{AppError, AppResult, ErrorCode, RunStartPreview};

/// Reading a repository for New run.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// Copying a large history takes a while.
const COPY_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The ref the bundle advertises, the only one.
pub const BUNDLE_REF: &str = "refs/heads/start";
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
}
