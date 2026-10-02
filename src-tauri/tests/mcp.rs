//! Agent access (SPEC.md, section 9): the MCP server driven as an agent
//! would drive it, over an in-memory stream, a Unix socket, and the real
//! `brainiac mcp` helper.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use brainiac_lib::git::GitService;
use brainiac_lib::mcp::{AgentServer, SOCKET_ENV};
use brainiac_lib::models::{AgentAccess, Settings, TaskFields, TaskStatus};
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
