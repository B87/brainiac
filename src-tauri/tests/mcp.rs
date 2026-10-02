//! Agent access (SPEC.md, section 9): the MCP server driven as an agent
//! would drive it, over an in-memory stream, a Unix socket, and the real
//! `brainiac mcp` helper.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::git::GitService;
use brainiac_lib::mcp::{AgentServer, SOCKET_ENV};
use brainiac_lib::models::{
    AgentAccess, NoteChangeOrigin, RevisionReason, Settings, TaskFields, TaskStatus,
};
use brainiac_lib::notes::KnowledgeEvent;
use brainiac_lib::tasks::TaskService;
use brainiac_lib::workspaces::RepositoryService;
use common::Harness;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{json, Value};
use tokio::sync::Notify;

/// An agent that counts `tools/list_changed` notifications.
#[derive(Clone, Default)]
struct Agent {
    list_changed: Arc<AtomicUsize>,
    notified: Arc<Notify>,
}

impl ClientHandler for Agent {
    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.list_changed.fetch_add(1, Ordering::SeqCst);
        self.notified.notify_one();
    }
}

type Client = RunningService<RoleClient, Agent>;

async fn server(h: &Harness, access: AgentAccess) -> Arc<AgentServer> {
    let emitter: brainiac_lib::workspaces::Emitter = Arc::new(|_| {});
    let repositories = Arc::new(RepositoryService::new(
        h.core.clone(),
        GitService::detect().await,
        Settings::default(),
        emitter,
    ));
    let tasks = Arc::new(TaskService::new(Arc::clone(&h.notes)));
    AgentServer::new(Arc::clone(&h.notes), tasks, repositories, access)
}

/// Connect an agent over an in-memory stream.
async fn connect(server: &Arc<AgentServer>) -> (Client, Agent) {
    let (server_side, agent_side) = tokio::io::duplex(1 << 16);
    let serving = Arc::clone(server);
    tokio::spawn(async move { serving.serve(server_side).await });
    let agent = Agent::default();
    let client = agent.clone().serve(agent_side).await.unwrap();
    (client, agent)
}

async fn tool_names(client: &Client) -> Vec<String> {
    let mut names: Vec<String> = client
        .list_tools(None)
        .await
        .unwrap()
        .tools
        .into_iter()
        .map(|t| t.name.into_owned())
        .collect();
    names.sort();
    names
}

/// Call a tool: its JSON on success, or the error text.
async fn call(client: &Client, name: &'static str, args: Value) -> Result<Value, String> {
    let mut params = CallToolRequestParams::new(name);
    params.arguments = args.as_object().cloned();
    let result = client.call_tool(params).await.unwrap();
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<String>();
    if result.is_error == Some(true) {
        Err(text)
    } else {
        Ok(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} returned {text}: {e}")))
    }
}

const READ_TOOLS: &[&str] = &[
    "get_repository_notes",
    "get_task",
    "get_today",
    "list_notes",
    "list_repositories",
    "list_tasks",
    "read_note",
    "repository_for_path",
    "search",
];

#[tokio::test(flavor = "multi_thread")]
async fn with_access_off_an_agent_sees_no_tools_and_is_told_where_to_turn_it_on() {
    let h = Harness::new(false).await;
    let server = server(&h, AgentAccess::Off).await;
    let (client, _) = connect(&server).await;

    let info = client.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().unwrap().name, "brainiac");
    assert!(info
        .instructions
        .as_deref()
        .is_some_and(|i| i.contains("Settings → Agent access")));
    assert!(tool_names(&client).await.is_empty());
    let err = call(&client, "search", json!({"query": "plan"}))
        .await
        .unwrap_err();
    assert!(err.starts_with("PERMISSION_DENIED"), "{err}");
    assert_eq!(server.connections(), 1);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_the_access_mode_updates_connected_agents() {
    let h = Harness::new(false).await;
    let server = server(&h, AgentAccess::Off).await;
    let (client, agent) = connect(&server).await;
    assert!(tool_names(&client).await.is_empty());

    server.set_access(AgentAccess::ReadOnly);
    tokio::time::timeout(Duration::from_secs(5), agent.notified.notified())
        .await
        .expect("the agent hears that its tools changed");
    assert_eq!(tool_names(&client).await, READ_TOOLS);

    // Setting the same mode again is not a change.
    server.set_access(AgentAccess::ReadOnly);
    server.set_access(AgentAccess::Off);
    tokio::time::timeout(Duration::from_secs(5), agent.notified.notified())
        .await
        .unwrap();
    assert_eq!(agent.list_changed.load(Ordering::SeqCst), 2);
    assert!(tool_names(&client).await.is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_schemas_carry_no_number_formats() {
    let h = Harness::new(false).await;
    let server = server(&h, AgentAccess::ReadOnly).await;
    let (client, _) = connect(&server).await;
    let tools = client.list_tools(None).await.unwrap().tools;
    let schemas = serde_json::to_string(&tools).unwrap();
    assert!(schemas.contains("\"limit\""), "{schemas}");
    assert!(!schemas.contains("\"format\""), "{schemas}");
    assert!(tools
        .iter()
        .all(|t| t.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true)));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_finds_and_reads_notes() {
    let h = Harness::new(false).await;
    h.write(
        "Projects/Parser.md",
        "# Parser\nThe parser needs fetch_with_backoff.\n",
    );
    h.write("Projects/diagram.png", "png");
    h.write("Inbox/Idea.md", "# Idea\nSee [[Parser]].\n");
    h.scan().await;
    let server = server(&h, AgentAccess::ReadOnly).await;
    let (client, _) = connect(&server).await;

    let found = call(&client, "search", json!({"query": "backoff"}))
        .await
        .unwrap();
    assert_eq!(found["notes_total"], 1);
    assert_eq!(found["notes"][0]["detail"], "Projects/Parser.md");
    assert!(found["notes"][0]["snippet"]
        .as_str()
        .unwrap()
        .contains("backoff"));
    let id = found["notes"][0]["id"].as_str().unwrap().to_string();

    let by_path = call(&client, "read_note", json!({"path": "Projects/Parser.md"}))
        .await
        .unwrap();
    assert_eq!(by_path["id"], id.as_str());
    assert_eq!(by_path["title"], "Parser");
    assert!(by_path["text"].as_str().unwrap().starts_with("# Parser"));
    assert_eq!(by_path["version"].as_str().unwrap().len(), 64);
    assert_eq!(by_path["linked_from"][0]["path"], "Inbox/Idea.md");

    // An absolute path inside the vault names the same note.
    let absolute = h.vault.join("Projects/Parser.md");
    let by_absolute = call(&client, "read_note", json!({"path": absolute}))
        .await
        .unwrap();
    assert_eq!(by_absolute["id"], id.as_str());
    let by_id = call(&client, "read_note", json!({"note_id": id}))
        .await
        .unwrap();
    assert_eq!(by_id["path"], "Projects/Parser.md");

    let err = call(&client, "read_note", json!({"path": "../outside.md"}))
        .await
        .unwrap_err();
    assert!(err.starts_with("VALIDATION"), "{err}");
    let err = call(&client, "read_note", json!({"path": "Projects/Nope.md"}))
        .await
        .unwrap_err();
    assert!(err.starts_with("NOT_FOUND"), "{err}");
    let err = call(&client, "read_note", json!({})).await.unwrap_err();
    assert!(err.starts_with("VALIDATION"), "{err}");

    let top = call(&client, "list_notes", json!({})).await.unwrap();
    assert_eq!(top["folders"], json!(["Inbox", "Projects"]));
    let projects = call(&client, "list_notes", json!({"folder": "Projects"}))
        .await
        .unwrap();
    assert_eq!(projects["notes"][0]["path"], "Projects/Parser.md");
    assert_eq!(projects["other_files"], json!(["Projects/diagram.png"]));

    // Reading is not opening: the user's recent notes stay theirs.
    h.notes.mark_opened(&id).await.unwrap();
    let recent = call(&client, "list_notes", json!({"recent": true}))
        .await
        .unwrap();
    assert_eq!(recent.as_array().unwrap().len(), 1);
    assert_eq!(recent[0]["path"], "Projects/Parser.md");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_sees_tasks_today_and_the_repository_it_works_in() {
    let h = Harness::new(false).await;
    h.write("Parser.md", "# Parser\n");
    h.scan().await;
    let note = common::note_id_at(&h, "Parser.md").await;
    let repo = h.register_repository("parser").await;
    h.notes.link_repository(&note, &repo).await.unwrap();
    let fields = |title: &str, status, planned: Option<&str>| TaskFields {
        title: title.into(),
        description: String::new(),
        status,
        planned_date: planned.map(Into::into),
        due_date: None,
        note_id: Some(note.clone()),
        repository_id: Some(repo.clone()),
        sorted: false,
    };
    // Planned for a past day, so it is in Today whatever the date.
    let open = h
        .tasks
        .create(fields(
            "Review the parser",
            TaskStatus::Todo,
            Some("2020-01-02"),
        ))
        .await
        .unwrap();
    h.tasks
        .create(fields("Old work", TaskStatus::Done, None))
        .await
        .unwrap();
    let server = server(&h, AgentAccess::ReadOnly).await;
    let (client, _) = connect(&server).await;

    let tasks = call(&client, "list_tasks", json!({})).await.unwrap();
    assert_eq!(tasks.as_array().unwrap().len(), 1, "open tasks by default");
    assert_eq!(tasks[0]["title"], "Review the parser");
    assert_eq!(tasks[0]["repository"]["name"], "parser");
    assert_eq!(tasks[0]["note"]["path"], "Parser.md");
    let all = call(&client, "list_tasks", json!({"statuses": ["todo", "done"]}))
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);

    let task = call(&client, "get_task", json!({"task_id": open.id}))
        .await
        .unwrap();
    assert_eq!(task["version"], open.version);
    assert_eq!(task["planned_date"], "2020-01-02");

    let today = call(&client, "get_today", json!({})).await.unwrap();
    assert_eq!(today["open"][0]["id"], open.id.as_str());
    assert_eq!(today["repositories"][0]["id"], repo.as_str());

    // From a folder inside the repository, as an agent's working directory.
    let root = h.tmp.path().join("repos/parser");
    std::fs::create_dir_all(root.join("src/lexer")).unwrap();
    let found = call(
        &client,
        "repository_for_path",
        json!({"path": root.join("src/lexer")}),
    )
    .await
    .unwrap();
    assert_eq!(found["id"], repo.as_str());
    assert_eq!(found["name"], "parser");
    let err = call(&client, "repository_for_path", json!({"path": h.vault}))
        .await
        .unwrap_err();
    assert!(err.starts_with("NOT_FOUND"), "{err}");
    let err = call(&client, "repository_for_path", json!({"path": "src"}))
        .await
        .unwrap_err();
    assert!(err.starts_with("VALIDATION"), "{err}");

    let repo_notes = call(
        &client,
        "get_repository_notes",
        json!({"repository_id": repo}),
    )
    .await
    .unwrap();
    assert_eq!(repo_notes["notes"][0]["path"], "Parser.md");
    assert_eq!(repo_notes["open_tasks"][0]["id"], open.id.as_str());
    let listed = call(&client, "list_repositories", json!({})).await.unwrap();
    assert_eq!(listed[0]["remote_url"], "git@example.com:team/parser.git");
    client.cancel().await.unwrap();
}

/// A short folder in `/tmp`: socket paths are limited to 104 bytes.
fn socket_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("bm")
        .tempdir_in("/tmp")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_helper_connects_an_agent_to_the_app_through_the_socket() {
    let h = Harness::new(false).await;
    h.write("Plan.md", "# Plan\nship it\n");
    h.scan().await;
    let server = server(&h, AgentAccess::ReadOnly).await;
    let dir = socket_dir();
    let socket = dir.path().join("mcp.sock");
    let listener = server.bind(&socket).await.unwrap();
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&socket).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    tokio::spawn(Arc::clone(&server).accept(listener));

    // The agent runs `brainiac mcp` and speaks MCP over its stdio.
    let mut helper = tokio::process::Command::new(env!("CARGO_BIN_EXE_brainiac"))
        .arg("mcp")
        .env(SOCKET_ENV, &socket)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdout = helper.stdout.take().unwrap();
    let stdin = helper.stdin.take().unwrap();
    let client = Agent::default().serve((stdout, stdin)).await.unwrap();
    let found = call(&client, "search", json!({"query": "ship"}))
        .await
        .unwrap();
    assert_eq!(found["notes"][0]["detail"], "Plan.md");
    assert_eq!(server.connections(), 1);

    // The agent quitting ends the helper cleanly.
    client.cancel().await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), helper.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    common::wait_for("the connection to close", || {
        let n = server.connections();
        async move { n == 0 }
    })
    .await;

    // A second app cannot take the socket over; quitting removes it.
    let other = server_without_vault().await;
    assert!(other.bind(&socket).await.is_err());
    server.close();
    assert!(!socket.exists());
}

async fn server_without_vault() -> Arc<AgentServer> {
    let h = Harness::new(false).await;
    server(&h, AgentAccess::Off).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_socket_is_replaced_and_a_missing_app_is_reported() {
    let h = Harness::new(false).await;
    let server = server(&h, AgentAccess::Off).await;
    let dir = socket_dir();
    let socket = dir.path().join("mcp.sock");
    // A socket left behind by an app that crashed.
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    server.bind(&socket).await.unwrap();

    // Not a socket: never removed.
    let file = dir.path().join("file.sock");
    std::fs::write(&file, "keep").unwrap();
    assert!(server.bind(&file).await.is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");

    // A development build is not in an app bundle, so it cannot open the app.
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_brainiac"))
        .arg("mcp")
        .env(SOCKET_ENV, dir.path().join("none.sock"))
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout carries only the protocol");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Brainiac is not running"), "{stderr}");
}

// ---------------------------------------------------------------------------
// Read and write
// ---------------------------------------------------------------------------

const WRITE_TOOLS: &[&str] = &[
    "complete_task",
    "create_note",
    "create_task",
    "edit_note",
    "link_repository",
    "unlink_repository",
    "update_task",
];

#[tokio::test(flavor = "multi_thread")]
async fn write_tools_are_listed_and_callable_only_in_read_and_write() {
    let h = Harness::new(false).await;
    let server = server(&h, AgentAccess::ReadOnly).await;
    let (client, agent) = connect(&server).await;
    let err = call(&client, "create_task", json!({"title": "x"}))
        .await
        .unwrap_err();
    assert!(err.starts_with("PERMISSION_DENIED"), "{err}");
    assert!(err.contains("read only"), "{err}");

    server.set_access(AgentAccess::ReadWrite);
    tokio::time::timeout(Duration::from_secs(5), agent.notified.notified())
        .await
        .unwrap();
    let mut all: Vec<&str> = READ_TOOLS.iter().chain(WRITE_TOOLS).copied().collect();
    all.sort();
    assert_eq!(tool_names(&client).await, all);
    let task = call(&client, "create_task", json!({"title": "x"}))
        .await
        .unwrap();
    assert_eq!(task["to_sort"], true);
    client.cancel().await.unwrap();
}

/// The roadmap's exit gate, without the UI: find a note, create and complete
/// a task, and link the note to a repository; the app hears every change.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_plans_and_completes_work_and_the_app_hears_it() {
    let h = Harness::new(false).await;
    h.write("Parser.md", "# Parser\nRewrite the tokenizer.\n");
    h.scan().await;
    let repo = h.register_repository("parser").await;
    let server = server(&h, AgentAccess::ReadWrite).await;
    let (client, _) = connect(&server).await;

    let found = call(&client, "search", json!({"query": "tokenizer"}))
        .await
        .unwrap();
    let note = found["notes"][0]["id"].as_str().unwrap().to_string();
    let linked = call(
        &client,
        "link_repository",
        json!({"path": "Parser.md", "repository_id": repo}),
    )
    .await
    .unwrap();
    assert_eq!(linked["note_id"], note.as_str());
    let context = h.notes.context(&note).await.unwrap();
    assert_eq!(context.repositories[0].repository_id, repo);

    let task = call(
        &client,
        "create_task",
        json!({
            "title": "Rewrite the tokenizer",
            "planned_date": "2026-10-03",
            "note_id": note,
            "repository_id": repo,
        }),
    )
    .await
    .unwrap();
    let task_id = task["id"].as_str().unwrap().to_string();
    assert_eq!(task["repository"]["name"], "parser");
    assert_eq!(task["note"]["path"], "Parser.md");
    let version = task["version"].as_i64().unwrap();

    let done = call(
        &client,
        "complete_task",
        json!({"task_id": task_id, "expected_version": version}),
    )
    .await
    .unwrap();
    assert_eq!(done["status"], "done");
    assert!(done["completed_at"].is_string());
    let stored = h.tasks.get(&task_id).await.unwrap();
    assert_eq!(stored.status, TaskStatus::Done);
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::TaskChanged(t)
        if t.task_id == task_id && t.version == Some(stored.version))));

    // The version is spent: completing again from the old read is a conflict.
    let err = call(
        &client,
        "complete_task",
        json!({"task_id": task_id, "expected_version": version}),
    )
    .await
    .unwrap_err();
    assert!(err.starts_with("CONFLICT"), "{err}");
    assert!(
        err.contains(&format!("current version is {}", stored.version)),
        "{err}"
    );

    let unlinked = call(
        &client,
        "unlink_repository",
        json!({"note_id": note, "repository_id": repo}),
    )
    .await
    .unwrap();
    assert_eq!(unlinked["linked"], false);
    assert!(h
        .notes
        .context(&note)
        .await
        .unwrap()
        .repositories
        .is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn update_task_changes_only_the_fields_it_names() {
    let h = Harness::new(false).await;
    let repo = h.register_repository("api").await;
    let server = server(&h, AgentAccess::ReadWrite).await;
    let (client, _) = connect(&server).await;
    let task = call(
        &client,
        "create_task",
        json!({"title": "Ship", "description": "carefully", "planned_date": "2026-10-05",
               "due_date": "2026-10-09", "repository_id": repo}),
    )
    .await
    .unwrap();
    let updated = call(
        &client,
        "update_task",
        json!({"task_id": task["id"], "expected_version": task["version"],
               "status": "in_progress", "planned_date": null}),
    )
    .await
    .unwrap();
    assert_eq!(updated["status"], "in_progress");
    assert!(updated.get("planned_date").is_none(), "null clears it");
    assert_eq!(updated["due_date"], "2026-10-09", "omitted stays");
    assert_eq!(updated["description"], "carefully");
    assert_eq!(updated["repository"]["id"], repo.as_str());
    assert_eq!(
        updated["to_sort"].as_bool(),
        None,
        "a dated task stays sorted"
    );

    let err = call(
        &client,
        "update_task",
        json!({"task_id": task["id"], "expected_version": task["version"], "title": "Late"}),
    )
    .await
    .unwrap_err();
    assert!(err.starts_with("CONFLICT"), "{err}");
    assert_eq!(
        h.tasks
            .get(task["id"].as_str().unwrap())
            .await
            .unwrap()
            .title,
        "Ship"
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_creates_a_note_with_its_text() {
    let h = Harness::new(false).await;
    let repo = h.register_repository("parser").await;
    let server = server(&h, AgentAccess::ReadWrite).await;
    let (client, _) = connect(&server).await;

    let note = call(
        &client,
        "create_note",
        json!({"title": "Parser decisions", "folder": "Projects",
               "text": "We keep the hand-written lexer.\n", "repository_id": repo}),
    )
    .await
    .unwrap();
    assert_eq!(note["path"], "Projects/Parser decisions.md");
    let id = note["id"].as_str().unwrap().to_string();
    let text = h.read("Projects/Parser decisions.md");
    assert_eq!(
        text,
        format!(
            "---\nbrainiac_id: {id}\n---\n# Parser decisions\n\nWe keep the hand-written lexer.\n"
        )
    );
    let read = call(&client, "read_note", json!({"note_id": id}))
        .await
        .unwrap();
    assert_eq!(read["version"], note["version"]);
    assert_eq!(read["repositories"][0]["id"], repo.as_str());
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::NoteChanged(n)
        if n.note_id == id && n.origin == NoteChangeOrigin::Agent)));

    // Its own heading and frontmatter stay; another note's identity does not.
    let copy = call(
        &client,
        "create_note",
        json!({"title": "Copy", "text": "---\ntags: [x]\nbrainiac_id: someone-else\n---\n## Own heading\n"}),
    )
    .await
    .unwrap();
    let copy_id = copy["id"].as_str().unwrap();
    assert_eq!(
        h.read("Copy.md"),
        format!("---\ntags: [x]\nbrainiac_id: {copy_id}\n---\n## Own heading\n")
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_edit_is_versioned_kept_in_history_and_leaves_the_users_draft_alone() {
    let h = Harness::new(false).await;
    h.write(
        "Plan.md",
        "---\nbrainiac_id: plan-1\n---\n# Plan\n- ship the parser\n- write docs\n",
    );
    h.scan().await;
    let server = server(&h, AgentAccess::ReadWrite).await;
    let (client, _) = connect(&server).await;
    let read = call(&client, "read_note", json!({"path": "Plan.md"}))
        .await
        .unwrap();
    let (id, v1) = (
        read["id"].as_str().unwrap().to_string(),
        read["version"].as_str().unwrap().to_string(),
    );
    // The user has unsaved edits open in the app.
    h.notes
        .keep_draft(&id, &v1, "# Plan\n- my unsaved idea\n".into())
        .await
        .unwrap();

    // Text that is not in the note, or is in it twice, changes nothing.
    for (old, why) in [("- deploy", "not in the note"), ("- ", "2 times")] {
        let err = call(
            &client,
            "edit_note",
            json!({"note_id": id, "expected_version": v1,
                   "edits": [{"old_text": old, "new_text": "x"}]}),
        )
        .await
        .unwrap_err();
        assert!(err.starts_with("VALIDATION") && err.contains(why), "{err}");
    }

    let edited = call(
        &client,
        "edit_note",
        json!({"note_id": id, "expected_version": v1,
               "edits": [{"old_text": "- write docs", "new_text": "- write docs\n- tag v0.3"}]}),
    )
    .await
    .unwrap();
    let v2 = edited["version"].as_str().unwrap().to_string();
    assert!(h.read("Plan.md").ends_with("- write docs\n- tag v0.3\n"));
    assert!(h.saw(|e| matches!(e, KnowledgeEvent::NoteChanged(n)
        if n.note_id == id && n.origin == NoteChangeOrigin::Agent && n.version.as_deref() == Some(v2.as_str()))));

    // A stale version is a conflict that names the current one.
    let err = call(
        &client,
        "edit_note",
        json!({"note_id": id, "expected_version": v1, "text": "# Plan\nnothing\n"}),
    )
    .await
    .unwrap_err();
    assert!(err.starts_with("CONFLICT") && err.contains(&v2), "{err}");

    // Whole text without the frontmatter keeps the note's identity.
    call(
        &client,
        "edit_note",
        json!({"path": "Plan.md", "expected_version": v2, "text": "# Plan\nall done\n"}),
    )
    .await
    .unwrap();
    assert_eq!(
        h.read("Plan.md"),
        "---\nbrainiac_id: plan-1\n---\n# Plan\nall done\n"
    );

    // Each agent save is its own revision, holding the text before it.
    let revisions = h.notes.revisions(&id).await.unwrap();
    assert_eq!(revisions.len(), 2, "{revisions:?}");
    assert!(revisions.iter().all(|r| r.reason == RevisionReason::Agent));
    let mut texts = Vec::new();
    for r in &revisions {
        texts.push(h.notes.revision_text(&r.id).await.unwrap());
    }
    assert!(texts
        .iter()
        .any(|t| t.ends_with("- ship the parser\n- write docs\n")));
    assert!(texts.iter().any(|t| t.ends_with("- tag v0.3\n")));

    // The user's draft is untouched, and now a conflict for the app to show.
    let content = h.notes.read(&id).await.unwrap();
    let draft = content.draft.expect("the draft is kept");
    assert_eq!(draft.text, "# Plan\n- my unsaved idea\n");
    assert_eq!(draft.base_version, v1);
    client.cancel().await.unwrap();
}
