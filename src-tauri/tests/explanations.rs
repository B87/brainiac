//! Explaining changes as the app does it (SPEC.md, section 14): the
//! once-per-repository question, Explain on a commit and a branch, the
//! explain run on the fake engine with its follow-up turn, the checked
//! explanation with its time and cost, the run discarded with its
//! conversation kept, notes placed again on a moved branch, known concepts,
//! Save as note, and Delete. The controller runs in this process.

mod fake_engine;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::agents::controller::state::StateDir;
use brainiac_lib::agents::controller::{serve, Config};
use brainiac_lib::agents::{
    image, AgentRunService, AgentSettingsService, RepositoryLookup, RunArtifacts, RunRuntime,
};
use brainiac_lib::credentials::{CommandRunner, CredentialService, MemoryStore};
use brainiac_lib::db::{self, Db};
use brainiac_lib::explain::service::{ExplanationService, PullRequestFacts};
use brainiac_lib::git::GitService;
use brainiac_lib::models::{
    AgentPayment, AgentProvider, AnswerCodeSharingRequest, CodeConsentState, ConceptKind,
    ErrorCode, ExplainDepth, ExplainSubject, ExplainSubjectKind, ExplanationRecord,
    ExplanationState, SaveAgentCredentialRequest, SecretSource, Settings, StartExplanationRequest,
};
use brainiac_lib::notes::{NoteService, Stores};
use brainiac_lib::sharing::CodeSharingService;
use brainiac_lib::workspaces::RepositoryService;
use fake_engine::{git, FakeEngine, KEY};
use serde_json::json;

const SOCKET: &str = "/fake/docker.sock";
const CLAUDE_CODE: &str = "claude-code";

struct Harness {
    tmp: tempfile::TempDir,
    repo: PathBuf,
    repository_id: String,
    engine: FakeEngine,
    _server: tokio::task::JoinHandle<()>,
    runs: Arc<AgentRunService>,
    explanations: Arc<ExplanationService>,
    sharing: Arc<CodeSharingService>,
    notes: Arc<NoteService>,
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn lines(n: usize, changed: &[(usize, &str)]) -> String {
    (1..=n)
        .map(|i| {
            changed
                .iter()
                .find(|(at, _)| *at == i)
                .map_or(format!("line {i}"), |(_, text)| text.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

impl Harness {
    async fn new() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();

        // main: a first commit, then one that changes line 5; a branch
        // `feature` off it changes line 8 and adds a file.
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        write(&repo, "src/lib.rs", &lines(12, &[]));
        write(&repo, "docs/rules.md", "# Rules\n\nThe limit is three.\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "First"]);
        write(&repo, "src/lib.rs", &lines(12, &[(5, "let limit = 3;")]));
        git(&repo, &["commit", "-q", "-am", "Set the limit"]);
        git(&repo, &["checkout", "-q", "-b", "feature"]);
        write(
            &repo,
            "src/lib.rs",
            &lines(12, &[(5, "let limit = 3;"), (8, "check(limit);")]),
        );
        write(&repo, "src/check.rs", "fn check() {}\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "Check the limit"]);
        git(&repo, &["checkout", "-q", "main"]);

        let core = Db::open(&data.join(db::CORE_FILE)).unwrap();
        let stores = Stores::open(&data, core.clone()).unwrap();
        let history = stores.history.clone();
        let notes = NoteService::new(stores, data.clone(), Arc::new(|_| {}));
        notes.disable_watching();
        notes
            .select_vault(vault.to_str().unwrap(), false)
            .await
            .unwrap();

        let repositories = Arc::new(RepositoryService::new(
            core.clone(),
            GitService::detect().await,
            Settings::default(),
            Arc::new(|_| {}),
        ));
        let repository_id = repositories.register(&repo).await.unwrap().id;

        let credentials = Arc::new(CredentialService::new(
            Arc::new(MemoryStore::default()),
            CommandRunner::new(data.join("commands")),
        ));
        let settings = Arc::new(AgentSettingsService::new(
            core.clone(),
            Arc::clone(&credentials),
        ));
        let recipe = image::recipe();
        core.call(move |conn| {
            conn.execute_batch(&format!(
                "UPDATE agent_hosts SET socket = '{SOCKET}', image_id = 'sha256:abc',
                   image_recipe = '{recipe}', image_built_at = '2026-10-05T00:00:00Z'
                 WHERE id = 'local';"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
        let profile = settings.profile(CLAUDE_CODE).await.unwrap();
        settings
            .save_credential(SaveAgentCredentialRequest {
                profile_id: CLAUDE_CODE.into(),
                expected_version: profile.version,
                payment: AgentPayment::ApiKey,
                source: SecretSource::Store,
                secret: Some(KEY.to_string()),
            })
            .await
            .unwrap();
        let revision = settings
            .profile(CLAUDE_CODE)
            .await
            .unwrap()
            .credential
            .revision;
        core.call(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO agent_tests (host_id, profile_id, passed_at,
                   credential_revision, image_id, engine_socket)
                 VALUES ('local', ?1, '2026-10-05T00:00:00Z', ?2, 'sha256:abc', ?3)",
                rusqlite::params![CLAUDE_CODE, revision, SOCKET],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let engine = FakeEngine::default();
        engine.get().volumes = tmp.path().join("volumes");
        let config = Config {
            state: StateDir::new(tmp.path().join("c")),
            socket: tmp.path().join("c").join("runner.sock"),
            idle_exit: None,
            spawn_guard: false,
            service: false,
        };
        let serving = engine.clone();
        let server = tokio::spawn(async move {
            serve(config, serving).await.unwrap();
        });

        let artifacts = Arc::new(RunArtifacts::new(
            Some(GitService::detect().await.unwrap()),
            &data,
        ));
        let (changes, _) = tokio::sync::broadcast::channel::<String>(256);
        let sender = changes.clone();
        let sharing = CodeSharingService::new(core.clone(), Arc::new(|| {}));
        let runs = AgentRunService::new(
            history.clone(),
            Arc::clone(&settings),
            Arc::clone(&sharing),
            credentials,
            Arc::clone(&artifacts),
            Arc::new(RunRuntime::attach(
                StateDir::new(tmp.path().join("c")),
                tmp.path().join("c").join("runner.sock"),
            )),
            Arc::clone(&repositories) as Arc<dyn RepositoryLookup>,
            Arc::new(move |event| {
                let _ = sender.send(event.run_id);
            }),
        );
        for _ in 0..100 {
            if runs.controller_status().await.running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let explanations = ExplanationService::new(
            core,
            history,
            Arc::clone(&runs),
            settings,
            Arc::clone(&sharing),
            artifacts,
            repositories,
            Arc::clone(&notes),
            Arc::new(|_| {}),
            changes,
        );
        Harness {
            tmp,
            repo,
            repository_id,
            engine,
            _server: server,
            runs,
            explanations,
            sharing,
            notes,
        }
    }

    fn head(&self, rev: &str) -> String {
        git(&self.repo, &["rev-parse", rev])
    }

    fn commit(&self, rev: &str) -> ExplainSubject {
        ExplainSubject {
            kind: ExplainSubjectKind::Commit,
            reference: self.head(rev),
        }
    }

    fn feature(&self) -> ExplainSubject {
        ExplainSubject {
            kind: ExplainSubjectKind::Branch,
            reference: "refs/heads/feature".into(),
        }
    }

    async fn allow(&self) {
        self.sharing
            .answer(AnswerCodeSharingRequest {
                repository_id: self.repository_id.clone(),
                provider: AgentProvider::Anthropic,
                allowed: true,
                workspace_id: None,
            })
            .await
            .unwrap();
    }

    async fn explain(
        &self,
        subject: ExplainSubject,
    ) -> brainiac_lib::models::AppResult<ExplanationRecord> {
        self.explanations
            .start(StartExplanationRequest {
                repository_id: self.repository_id.clone(),
                subject,
                profile_id: CLAUDE_CODE.into(),
                host_id: "local".into(),
                depth: ExplainDepth::TeachMe,
                questions: true,
            })
            .await
    }

    async fn ended(&self, id: &str) -> ExplanationRecord {
        for _ in 0..750 {
            let record = self.explanations.get(id).await.unwrap();
            if record.state != ExplanationState::Working {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "the explanation did not end: {:?}",
            self.explanations.get(id).await.unwrap()
        );
    }

    /// The explain run discarded, once its run service saw it end.
    async fn discarded(&self, run_id: &str) {
        for _ in 0..500 {
            if self.engine.get().discarded.iter().any(|r| r == run_id) {
                let run = self.runs.get(run_id).await.unwrap();
                if !run.kept {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the explain run was not discarded");
    }
}

fn quote(path: &str, start: u32, end: u32, text: &str) -> serde_json::Value {
    json!({ "path": path, "start": start, "end": end, "quote": text })
}

/// An explanation of the commit that sets the limit, with one note on line
/// 5 and its quotes (the second cited at the wrong lines).
fn good_commit_file() -> String {
    json!({
        "summary": "Sets the limit the docs promise.",
        "sources_read": ["docs/rules.md"],
        "tour": [{ "path": "src/lib.rs", "role": "the rule" }],
        "notes": [{
            "path": "src/lib.rs", "new_start": 5, "new_end": 5,
            "text": "The limit, as the docs say.",
            "sources": [
                quote("src/lib.rs", 5, 5, "let limit = 3;"),
                quote("docs/rules.md", 1, 1, "The limit is three."),
                quote("docs/rules.md", 1, 1, "Not in the file."),
            ],
        }],
        "concepts": [
            { "name": "Limit rule", "kind": "project_pattern", "explanation": "Limits live in docs.", "appears": [{ "path": "src/lib.rs", "line": 5 }] },
            { "name": "let", "kind": "language", "explanation": "Binds a name.", "appears": [] },
        ],
        "questions": [{ "question": "What is the limit?", "answer": "Three." }],
        "disagreements": [],
        "known_used": ["Limit Rule (project pattern)", "no such concept"],
    })
    .to_string()
}

#[tokio::test]
async fn a_commit_is_explained_after_one_follow_up_turn() {
    let h = Harness::new().await;
    let subject = h.commit("main");

    // The first Explain in a repository asks before any code is sent.
    let dialog = h
        .explanations
        .dialog(&h.repository_id, subject.clone())
        .await
        .unwrap();
    assert_eq!(dialog.title, "Set the limit");
    assert_eq!(dialog.profile_id.as_deref(), Some(CLAUDE_CODE));
    let anthropic = dialog
        .consents
        .iter()
        .find(|c| c.provider == AgentProvider::Anthropic)
        .unwrap();
    assert_eq!(anthropic.state, CodeConsentState::Unasked);
    assert!(dialog.estimates.is_empty());
    let refused = h.explain(subject.clone()).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    assert_eq!(h.engine.get().launches, 0);
    h.allow().await;

    // The first file puts a note off the change; the follow-up fixes it.
    let off = good_commit_file().replace(
        "\"new_end\":5,\"new_start\":5",
        "\"new_end\":2,\"new_start\":1",
    );
    assert_ne!(off, good_commit_file());
    {
        let mut engine = h.engine.get();
        engine.explanations.push_back(off);
        engine.explanations.push_back(good_commit_file());
        engine.cost = Some(0.42);
    }
    let started = h.explain(subject.clone()).await.unwrap();
    assert_eq!(started.state, ExplanationState::Working);
    assert_eq!(started.model, "sonnet");
    let record = h.ended(&started.id).await;
    assert_eq!(record.state, ExplanationState::Ready, "{record:?}");

    let prompts = h.engine.get().prompts.clone();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[0].contains(".brainiac/explanation.json"));
    assert!(prompts[0].contains(&format!(
        "git diff {}..{}",
        h.head("main~1"),
        h.head("main")
    )));
    assert!(prompts[1].contains("no line there is changed on the new side"));

    let e = record.explanation.as_ref().unwrap();
    assert_eq!(e.notes.len(), 1);
    let note = &e.notes[0];
    assert_eq!((note.start, note.end), (5, 5));
    assert_eq!(note.sources.len(), 2);
    assert_eq!(note.sources_dropped, 1);
    assert!(note.sources[1].moved);
    assert_eq!(e.checks.moved, 1);
    assert_eq!(e.checks.left_out.len(), 1);
    assert_eq!(record.cost.as_ref().unwrap().micros, 420_000);
    assert!(record.duration_secs.is_some());
    assert!(record.errors.is_empty());

    // The explain run is gone from the engine, kept as a conversation, and
    // never listed in Runs.
    let run_id = record.run_id.clone().unwrap();
    h.discarded(&run_id).await;
    assert!(h.runs.list().await.unwrap().runs.is_empty());
    let events = h.runs.events(&run_id, 0).await.unwrap().events;
    assert!(!events.is_empty());
    assert!(h
        .engine
        .get()
        .reads
        .iter()
        .all(|p| p == "/workspace/.brainiac/explanation.json"));

    // A commit never moves, and the estimate now has one explanation.
    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(!placed.moved && !placed.gone);
    let dialog = h
        .explanations
        .dialog(&h.repository_id, subject.clone())
        .await
        .unwrap();
    assert_eq!(dialog.existing.len(), 1);
    let estimate = dialog
        .estimates
        .iter()
        .find(|e| e.depth == ExplainDepth::TeachMe)
        .unwrap();
    assert_eq!(estimate.cost.as_ref().unwrap().micros, 420_000);
    assert_eq!(estimate.from, 1);

    // Got it on a project pattern is told to this repository's prompts.
    let concept = h
        .explanations
        .learn(
            &h.repository_id,
            ConceptKind::ProjectPattern,
            "Limit rule",
            None,
        )
        .await
        .unwrap();
    h.explanations
        .learn(
            &h.repository_id,
            ConceptKind::Language,
            "let",
            Some(&record.id),
        )
        .await
        .unwrap();
    let view = h.explanations.settings_view().await.unwrap();
    assert_eq!(view.concepts.len(), 2);
    // Learned from the explanation, it keeps its words and where it was.
    let known = view.concepts.iter().find(|c| c.name == "let").unwrap();
    assert_eq!(known.description, "Binds a name.");
    assert_eq!(known.learned_from, record.tip[..7]);
    assert_eq!(known.learned_in_name.as_deref(), Some("repo"));
    assert_eq!(known.explanation_id.as_deref(), Some(record.id.as_str()));
    assert_eq!(view.stored.len(), 1);
    assert_eq!(view.stored_cost[0].micros, 420_000);

    // Save as note writes a linked Markdown copy into the vault.
    let note = h.explanations.save_as_note(&record.id).await.unwrap();
    assert!(
        note.relative_path.starts_with("Explanations/repo/"),
        "{}",
        note.relative_path
    );
    let text =
        std::fs::read_to_string(h.notes.vault().unwrap().root.join(&note.relative_path)).unwrap();
    assert!(text.contains("repository: \"repo\""));
    assert!(text.contains("The limit, as the docs say."));
    assert!(text.contains("Sources: `src/lib.rs:5`"));

    // Explain again replaces it; the run is given the known concepts. This
    // time the agent leaves "let" out of its concepts and says so, and keeps
    // "Limit rule" (named in `known_used` too: a new use, so not counted).
    let mut again_file: serde_json::Value = serde_json::from_str(&good_commit_file()).unwrap();
    again_file["concepts"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["name"] != "let");
    again_file["known_used"] = json!(["let (language)", "Limit rule", "no such concept"]);
    h.engine
        .get()
        .explanations
        .push_back(again_file.to_string());
    let again = h.explain(subject.clone()).await.unwrap();
    let again = h.ended(&again.id).await;
    assert_eq!(again.state, ExplanationState::Ready);
    let last = h.engine.get().prompts.last().cloned().unwrap();
    // The names are in a file the container gets, never in the prompt.
    assert!(last.contains("already knows 2 concepts"));
    assert!(last.contains("/opt/brainiac/input/known-concepts.tsv"));
    assert!(!last.contains("Limit rule"));
    let file = h
        .engine
        .get()
        .known_concepts
        .last()
        .cloned()
        .flatten()
        .expect("the run's container is given the known concepts");
    assert!(file.contains("limit rule\tLimit rule\tproject pattern\t"));
    assert!(file.contains("let\tlet\tlanguage\tBinds a name."));
    // Of the three names, one is not in the ledger and one was kept: only "let" counts.
    let left_out = &again.explanation.as_ref().unwrap().known_left_out;
    assert_eq!(left_out.len(), 1);
    assert_eq!(left_out[0].name, "let");
    assert!(h.explanations.get(&record.id).await.is_err());
    h.explanations.forget_concept(&concept).await.unwrap();

    // Delete takes the conversation with it.
    let again_run = again.run_id.clone().unwrap();
    h.discarded(&again_run).await;
    h.explanations.delete(&again.id).await.unwrap();
    assert!(h.runs.get(&again_run).await.is_err());
    assert!(h
        .explanations
        .for_subject(&h.repository_id, subject)
        .await
        .unwrap()
        .is_empty());
    drop(h.tmp);
}

#[tokio::test]
async fn a_file_that_fails_the_schema_twice_fails_with_its_errors() {
    let h = Harness::new().await;
    h.allow().await;
    {
        let mut engine = h.engine.get();
        engine.explanations.push_back("{\"summary\": 1}".into());
        engine.explanations.push_back("not json".into());
    }
    let started = h.explain(h.commit("main")).await.unwrap();
    let record = h.ended(&started.id).await;
    assert_eq!(record.state, ExplanationState::Failed);
    assert!(record.error.unwrap().contains("schema"));
    assert!(!record.errors.is_empty());
    assert!(record.explanation.is_none());
    h.discarded(&record.run_id.unwrap()).await;
}

#[tokio::test]
async fn a_repository_answered_no_is_never_explained() {
    let h = Harness::new().await;
    h.sharing
        .answer(AnswerCodeSharingRequest {
            repository_id: h.repository_id.clone(),
            provider: AgentProvider::Anthropic,
            allowed: false,
            workspace_id: None,
        })
        .await
        .unwrap();
    let refused = h.explain(h.commit("main")).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::Validation);
    assert_eq!(h.engine.get().launches, 0);
}

#[tokio::test]
async fn a_branch_explanation_follows_the_branch_when_it_moves() {
    let h = Harness::new().await;
    h.allow().await;
    let file = json!({
        "summary": "Checks the limit.",
        "tour": [
            { "path": "src/lib.rs", "role": "the call" },
            { "path": "src/check.rs", "role": "the check" },
        ],
        "notes": [{
            "path": "src/lib.rs", "new_start": 8, "new_end": 8,
            "text": "The new check.",
            "sources": [quote("src/lib.rs", 8, 8, "check(limit);")],
        }],
        "concepts": [],
    })
    .to_string();
    h.engine.get().explanations.push_back(file);
    let started = h.explain(h.feature()).await.unwrap();
    assert_eq!(started.title, "feature");
    assert_eq!(started.base, h.head("main"));
    let record = h.ended(&started.id).await;
    assert_eq!(record.state, ExplanationState::Ready, "{record:?}");
    assert_eq!(h.engine.get().prompts.len(), 1);

    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(!placed.moved);

    // Two lines go in above the note's line, and a file is added: the note
    // moves with its line, and the new file is not in the explanation.
    git(&h.repo, &["checkout", "-q", "feature"]);
    write(
        &h.repo,
        "src/lib.rs",
        &(String::from("new a\nnew b\n")
            + &lines(12, &[(5, "let limit = 3;"), (8, "check(limit);")])),
    );
    write(&h.repo, "src/more.rs", "fn more() {}\n");
    git(&h.repo, &["add", "."]);
    git(&h.repo, &["commit", "-q", "-m", "More"]);
    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(placed.moved && !placed.gone);
    assert_eq!(placed.notes.len(), 1);
    assert_eq!(
        (placed.notes[0].start, placed.notes[0].out_of_date),
        (10, false)
    );
    assert_eq!(placed.uncovered, ["src/more.rs"]);

    // The note's line changes: it is out of date.
    write(
        &h.repo,
        "src/lib.rs",
        &lines(12, &[(5, "let limit = 3;"), (8, "check(limit, 2);")]),
    );
    git(&h.repo, &["commit", "-q", "-am", "Change the call"]);
    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(placed.notes[0].out_of_date);

    // Merged into main: no changes against it, and the explanation is kept.
    git(&h.repo, &["checkout", "-q", "main"]);
    git(&h.repo, &["merge", "-q", "--ff-only", "feature"]);
    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(placed.gone);
    assert!(h.explanations.get(&record.id).await.is_ok());
}

/// A pull request of `feature` into main, as the pull request service would
/// describe it; the test changes it as the pull request moves.
fn pull_request(h: &Harness) -> Arc<std::sync::Mutex<PullRequestFacts>> {
    let facts = Arc::new(std::sync::Mutex::new(PullRequestFacts {
        repository_id: h.repository_id.clone(),
        number: 7,
        title: "Check the limit".into(),
        head_sha: h.head("feature"),
        base_sha: h.head("main"),
        source_branch: "feature".into(),
        target_branch: "main".into(),
        from_fork: false,
        mine: false,
        author: "Ana".into(),
    }));
    let reading = Arc::clone(&facts);
    h.explanations
        .set_pull_requests(Arc::new(move |_reference: String| {
            let facts = reading.lock().unwrap().clone();
            Box::pin(async move { Ok(facts) })
        }));
    facts
}

fn pull_request_subject() -> ExplainSubject {
    ExplainSubject {
        kind: ExplainSubjectKind::PullRequest,
        reference: "github.com/acme/api#7".into(),
    }
}

fn feature_file() -> String {
    json!({
        "summary": "Checks the limit.",
        "tour": [
            { "path": "src/lib.rs", "role": "the call" },
            { "path": "src/check.rs", "role": "the check" },
        ],
        "notes": [{
            "path": "src/lib.rs", "new_start": 8, "new_end": 8,
            "text": "The new check.",
            "sources": [quote("src/lib.rs", 8, 8, "check(limit);")],
        }],
        "concepts": [],
    })
    .to_string()
}

#[tokio::test]
async fn a_pull_request_and_its_branch_share_one_explanation() {
    let h = Harness::new().await;
    h.allow().await;
    let facts = pull_request(&h);

    // The branch is explained first; the pull request of the same commits
    // shows that explanation, and its dialog lists it.
    h.engine.get().explanations.push_back(feature_file());
    let started = h.explain(h.feature()).await.unwrap();
    let branch = h.ended(&started.id).await;
    assert_eq!(branch.state, ExplanationState::Ready, "{branch:?}");
    let shown = h
        .explanations
        .for_subject(&h.repository_id, pull_request_subject())
        .await
        .unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].id, branch.id);
    let dialog = h
        .explanations
        .dialog(&h.repository_id, pull_request_subject())
        .await
        .unwrap();
    assert_eq!(dialog.title, "#7 Check the limit");
    assert_eq!(dialog.existing.len(), 1);
    assert_eq!(dialog.blocked, None);
    // Someone else's head: the dialog says whose.
    assert_eq!(dialog.head_author.as_deref(), Some("Ana"));

    // Explained as a pull request too: compared with its target from their
    // merge base, the same changes, and shown on the branch as well.
    h.engine.get().explanations.push_back(feature_file());
    let started = h.explain(pull_request_subject()).await.unwrap();
    assert_eq!(started.title, "#7 Check the limit");
    assert_eq!(started.base, h.head("main"));
    let record = h.ended(&started.id).await;
    assert_eq!(record.state, ExplanationState::Ready, "{record:?}");
    let prompt = h.engine.get().prompts.last().cloned().unwrap();
    assert!(prompt.contains("pull request #7"));
    let on_branch = h
        .explanations
        .for_subject(&h.repository_id, h.feature())
        .await
        .unwrap();
    assert_eq!(on_branch.len(), 2);
    assert!(on_branch.iter().any(|r| r.id == record.id));

    // A push moves the head: the note follows its line, and the new file is
    // not in the explanation.
    git(&h.repo, &["checkout", "-q", "feature"]);
    write(&h.repo, "src/more.rs", "fn more() {}\n");
    git(&h.repo, &["add", "."]);
    git(&h.repo, &["commit", "-q", "-m", "More"]);
    git(&h.repo, &["checkout", "-q", "main"]);
    facts.lock().unwrap().head_sha = h.head("feature");
    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(placed.moved && !placed.gone);
    assert!(!placed.notes[0].out_of_date);
    assert_eq!(placed.uncovered, ["src/more.rs"]);

    // A new head not on this Mac: the notes cannot be placed, and the
    // dialog asks for a fetch first.
    facts.lock().unwrap().head_sha = "0123456789abcdef0123456789abcdef01234567".into();
    let placed = h.explanations.placement(&record.id).await.unwrap();
    assert!(placed.moved && !placed.gone);
    assert!(placed.notes.iter().all(|n| n.out_of_date));
    let dialog = h
        .explanations
        .dialog(&h.repository_id, pull_request_subject())
        .await
        .unwrap();
    assert!(dialog.fetch_first);
    assert!(dialog.blocked.unwrap().contains("fetch"));
    let refused = h.explain(pull_request_subject()).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn a_pull_request_from_a_fork_is_not_explained() {
    let h = Harness::new().await;
    h.allow().await;
    let facts = pull_request(&h);
    facts.lock().unwrap().from_fork = true;
    let dialog = h
        .explanations
        .dialog(&h.repository_id, pull_request_subject())
        .await
        .unwrap();
    assert!(dialog.blocked.unwrap().contains("fork"));
    assert!(!dialog.fetch_first);
    let refused = h.explain(pull_request_subject()).await.unwrap_err();
    assert_eq!(refused.code, ErrorCode::Validation);
    assert_eq!(h.engine.get().launches, 0);
}
