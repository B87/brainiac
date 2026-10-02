//! The tools agents call, and their shapes (docs/architecture.md, Agent
//! access). These shapes are a contract with agents, not the frontend's DTOs:
//! smaller, and documented by their doc comments, which become the schemas'
//! descriptions.

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{tool, tool_router, ErrorData, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AgentServer;
use crate::models::{
    AgentAccess, AppError, AppResult, NoteSummary, RepositorySummary, SearchKind, SearchRequest,
    Task, TaskFilter, TaskStatus, TextPart,
};

/// What the server tells every agent when it connects.
const INSTRUCTIONS: &str = include_str!("instructions.md");

/// Tools that change something; listed only in Read and write.
const WRITE_TOOLS: &[&str] = &[];

/// One agent's connection. `Clone` is cheap: the server is shared.
#[derive(Clone)]
pub struct Connection {
    server: Arc<AgentServer>,
    router: ToolRouter<Self>,
}

impl Connection {
    pub fn new(server: Arc<AgentServer>) -> Self {
        Self {
            server,
            router: Self::router(),
        }
    }

    /// Whether the access mode allows this tool now.
    fn allowed(&self, tool: &str) -> bool {
        match self.server.access() {
            AgentAccess::Off => false,
            AgentAccess::ReadOnly => !WRITE_TOOLS.contains(&tool),
            AgentAccess::ReadWrite => true,
        }
    }

    /// The registered repositories' names, to name each task's repository.
    async fn repository_names(&self) -> HashMap<String, String> {
        self.server
            .repositories
            .list()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.id, r.name))
            .collect()
    }
}

impl ServerHandler for Connection {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new("brainiac", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .router
            .list_all()
            .into_iter()
            .filter(|t| self.allowed(&t.name))
            .map(without_number_formats)
            .collect();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Checked again here: a call can race a change of the access mode.
        if !self.allowed(&request.name) && self.router.has_route(&request.name) {
            let text = match self.server.access() {
                AgentAccess::Off => "PERMISSION_DENIED: Agent access is off. The user can turn it on in Brainiac → Settings → Agent access.",
                _ => "PERMISSION_DENIED: Agent access is read only. The user can allow changes in Brainiac → Settings → Agent access.",
            };
            return Ok(CallToolResult::error(vec![ContentBlock::text(text)]).into());
        }
        let call = ToolCallContext::new(self, request, context);
        self.router.call(call).await
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        // Tell the agent whenever Settings → Agent access changes what it may use.
        let mut access = self.server.watch_access();
        let peer = context.peer;
        tokio::spawn(async move {
            while access.changed().await.is_ok() {
                if peer.notify_tool_list_changed().await.is_err() {
                    break;
                }
            }
        });
    }
}

/// Drop `format` from numeric schema properties: `schemars` writes formats
/// such as `uint32`, which Claude Code warns about (spike S5).
fn without_number_formats(mut tool: Tool) -> Tool {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                let numeric = matches!(
                    map.get("type").and_then(|t| t.as_str()),
                    Some("integer" | "number")
                ) || map
                    .get("type")
                    .and_then(|t| t.as_array())
                    .is_some_and(|ts| ts.iter().any(|t| t == "integer" || t == "number"));
                if numeric {
                    map.remove("format");
                }
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut schema = serde_json::Value::Object((*tool.input_schema).clone());
    strip(&mut schema);
    if let serde_json::Value::Object(map) = schema {
        tool.input_schema = Arc::new(map);
    }
    tool
}

// ---------------------------------------------------------------------------
// Results and errors
// ---------------------------------------------------------------------------

/// A tool's reply: compact JSON, or an error that starts with its code.
type Reply = Result<String, String>;

fn reply<T: Serialize>(result: AppResult<T>) -> Reply {
    match result {
        Ok(value) => serde_json::to_string(&value).map_err(|e| format!("IO: {e}")),
        Err(e) => Err(failure(&e)),
    }
}

fn failure(e: &AppError) -> String {
    let code = serde_json::to_value(e.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!("{code}: {}", e.message)
}

fn joined(parts: &[TextPart]) -> String {
    parts.iter().map(|p| p.text.as_str()).collect()
}

fn is_false(b: &bool) -> bool {
    !b
}

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

/// A note, briefly.
#[derive(Serialize)]
struct NoteOut {
    id: String,
    title: String,
    /// Path inside the vault.
    path: String,
    modified_at: String,
    /// The file is gone; the note keeps its tasks and links.
    #[serde(skip_serializing_if = "is_false")]
    missing: bool,
}

impl From<NoteSummary> for NoteOut {
    fn from(n: NoteSummary) -> Self {
        Self {
            id: n.id,
            title: n.title,
            path: n.relative_path,
            modified_at: n.modified_at,
            missing: n.missing,
        }
    }
}

#[derive(Serialize)]
struct NoteRef {
    id: String,
    title: String,
    path: String,
    #[serde(skip_serializing_if = "is_false")]
    missing: bool,
}

#[derive(Serialize)]
struct RepositoryRef {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct TaskOut {
    id: String,
    title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    status: TaskStatus,
    /// `YYYY-MM-DD`, the day the user plans to work on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    planned_date: Option<String>,
    /// `YYYY-MM-DD`, the deadline.
    #[serde(skip_serializing_if = "Option::is_none")]
    due_date: Option<String>,
    /// No date and not marked sorted yet.
    #[serde(skip_serializing_if = "is_false")]
    to_sort: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<NoteRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<RepositoryRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    /// Name it when changing the task.
    version: i64,
}

fn task_out(t: Task, names: &HashMap<String, String>) -> TaskOut {
    TaskOut {
        repository: t.repository_id.map(|id| RepositoryRef {
            name: names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "(removed)".into()),
            id,
        }),
        note: t.note.map(|n| NoteRef {
            id: n.id,
            title: n.title,
            path: n.relative_path,
            missing: n.missing,
        }),
        id: t.id,
        title: t.title,
        description: t.description,
        status: t.status,
        planned_date: t.planned_date,
        due_date: t.due_date,
        to_sort: t.to_sort,
        completed_at: t.completed_at,
        version: t.version,
    }
}

fn tasks_out(tasks: Vec<Task>, names: &HashMap<String, String>) -> Vec<TaskOut> {
    tasks.into_iter().map(|t| task_out(t, names)).collect()
}

/// A registered repository with the state Brainiac last observed.
#[derive(Serialize)]
struct RepositoryOut {
    id: String,
    name: String,
    path: String,
    /// `fresh`, `stale`, `refreshing`, `missing`, or `error`.
    state: crate::models::RepositoryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    /// Distinct changed paths, staged or not.
    #[serde(skip_serializing_if = "Option::is_none")]
    changed_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicted_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ahead: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    behind: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fetched_at: Option<String>,
}

impl From<RepositorySummary> for RepositoryOut {
    fn from(r: RepositorySummary) -> Self {
        Self {
            id: r.id,
            name: r.name,
            path: r.display_path,
            state: r.state,
            branch: r.head.and_then(|h| h.branch),
            changed_files: r.counts.map(|c| c.unique_paths),
            conflicted_files: r.counts.map(|c| c.conflicted).filter(|n| *n > 0),
            ahead: r.upstream.as_ref().map(|u| u.ahead),
            behind: r.upstream.as_ref().map(|u| u.behind),
            upstream: r.upstream.map(|u| u.ref_name),
            remote_url: r.remote_url,
            checked_at: r.last_checked_at,
            fetched_at: r.last_fetch_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// A task's status.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum StatusArg {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl From<StatusArg> for TaskStatus {
    fn from(s: StatusArg) -> Self {
        match s {
            StatusArg::Todo => TaskStatus::Todo,
            StatusArg::InProgress => TaskStatus::InProgress,
            StatusArg::Done => TaskStatus::Done,
            StatusArg::Cancelled => TaskStatus::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum KindArg {
    Note,
    Task,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchArgs {
    /// Keywords. Every word must match, also as the start of a longer word;
    /// "quoted words" must match in order. Not a question: use a few
    /// distinctive words.
    query: String,
    /// Only notes or only tasks; both when omitted.
    kinds: Option<Vec<KindArg>>,
    /// Results per kind, 1 to 50 (default 10).
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NoteArgs {
    /// The note's ID.
    note_id: Option<String>,
    /// Or its path: inside the vault (`Projects/Plan.md`) or absolute.
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListNotesArgs {
    /// A folder inside the vault, such as `Projects`; the top folder when omitted.
    folder: Option<String>,
    /// List the notes the user opened most recently instead of a folder.
    recent: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListTasksArgs {
    /// Only these statuses; open tasks (todo, in_progress) when omitted.
    statuses: Option<Vec<StatusArg>>,
    /// Only tasks still to sort (true) or only sorted ones (false).
    to_sort: Option<bool>,
    /// Only tasks linked to this note.
    note_id: Option<String>,
    /// Only tasks linked to this repository.
    repository_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TaskArgs {
    task_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PathArgs {
    /// An absolute folder or file path, such as the current working directory.
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RepositoryArgs {
    repository_id: String,
}

// ---------------------------------------------------------------------------
// Read tools
// ---------------------------------------------------------------------------

#[tool_router(router = router)]
impl Connection {
    #[tool(
        name = "search",
        description = "Keyword search over the user's notes (titles, text, code blocks) and tasks. Returns IDs, titles, paths or task details, and a snippet.",
        annotations(read_only_hint = true)
    )]
    async fn search(&self, Parameters(args): Parameters<SearchArgs>) -> Reply {
        let request = SearchRequest {
            query: args.query,
            kinds: args.kinds.map(|ks| {
                ks.into_iter()
                    .map(|k| match k {
                        KindArg::Note => SearchKind::Note,
                        KindArg::Task => SearchKind::Task,
                    })
                    .collect()
            }),
            limit: Some(args.limit.unwrap_or(10).clamp(1, 50)),
        };
        let results = match crate::index::search(&self.server.notes, request).await {
            Ok(r) => r,
            Err(e) => return Err(failure(&e)),
        };
        #[derive(Serialize)]
        struct Hit {
            id: String,
            title: String,
            /// The note's path, or the task's status and dates.
            detail: String,
            snippet: String,
        }
        #[derive(Serialize)]
        struct Out {
            notes: Vec<Hit>,
            notes_total: u64,
            tasks: Vec<Hit>,
            tasks_total: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            warning: Option<String>,
        }
        let hits = |group: crate::models::SearchGroup| -> Vec<Hit> {
            group
                .hits
                .into_iter()
                .map(|h| Hit {
                    title: joined(&h.title),
                    snippet: joined(&h.snippet),
                    id: h.id,
                    detail: h.detail,
                })
                .collect()
        };
        let warning = match results.index.state {
            crate::models::IndexState::Ready => None,
            crate::models::IndexState::Indexing => Some(format!(
                "Notes are still being indexed ({} of {}); results may be incomplete.",
                results.index.done, results.index.total
            )),
            crate::models::IndexState::NoVault => {
                Some("No vault is chosen yet; only tasks were searched.".into())
            }
            crate::models::IndexState::Unavailable => Some(
                results
                    .index
                    .message
                    .clone()
                    .unwrap_or_else(|| "Note search is unavailable.".into()),
            ),
        };
        reply(Ok(Out {
            notes_total: results.notes.total,
            tasks_total: results.tasks.total,
            notes: hits(results.notes),
            tasks: hits(results.tasks),
            warning,
        }))
    }

    #[tool(
        name = "read_note",
        description = "Read a note's Markdown, its version, the repositories it links to, its tasks, and the notes that link to it. Name the note by ID or by path.",
        annotations(read_only_hint = true)
    )]
    async fn read_note(&self, Parameters(args): Parameters<NoteArgs>) -> Reply {
        let id = match (args.note_id, args.path) {
            (Some(id), None) => id,
            (None, Some(path)) => self
                .server
                .notes
                .note_id_at(&path)
                .await
                .map_err(|e| failure(&e))?,
            _ => return Err("VALIDATION: Give either note_id or path.".into()),
        };
        let content = self.server.notes.read(&id).await.map_err(|e| failure(&e))?;
        let context = self
            .server
            .notes
            .context(&id)
            .await
            .map_err(|e| failure(&e))?;
        let names = self.repository_names().await;
        #[derive(Serialize)]
        struct Linked {
            id: String,
            name: String,
            /// False when the repository was removed from Brainiac.
            registered: bool,
        }
        #[derive(Serialize)]
        struct Out {
            #[serde(flatten)]
            note: NoteOut,
            /// Name it when editing the note.
            version: String,
            /// The Markdown; absent for a file that is not text or is over 5 MiB.
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<String>,
            /// The user has unsaved edits to this note open in Brainiac.
            #[serde(skip_serializing_if = "is_false")]
            unsaved_edits_in_app: bool,
            repositories: Vec<Linked>,
            tasks: Vec<TaskOut>,
            linked_from: Vec<NoteOut>,
        }
        reply(Ok(Out {
            version: content.version,
            text: content.text,
            unsaved_edits_in_app: content.draft.is_some(),
            note: content.note.into(),
            repositories: context
                .repositories
                .into_iter()
                .map(|r| Linked {
                    name: names.get(&r.repository_id).cloned().unwrap_or(r.name),
                    id: r.repository_id,
                    registered: r.registered,
                })
                .collect(),
            tasks: tasks_out(context.tasks, &names),
            linked_from: context
                .backlinks
                .into_iter()
                .map(|b| b.note.into())
                .collect(),
        }))
    }

    #[tool(
        name = "list_notes",
        description = "List one folder of the vault (its subfolders, notes, and other files), or the notes the user opened most recently.",
        annotations(read_only_hint = true)
    )]
    async fn list_notes(&self, Parameters(args): Parameters<ListNotesArgs>) -> Reply {
        if args.recent == Some(true) {
            let lists = self.server.notes.lists().await;
            return reply(
                lists.map(|l| l.recent.into_iter().map(NoteOut::from).collect::<Vec<_>>()),
            );
        }
        let listing = self
            .server
            .notes
            .list_folder(args.folder)
            .await
            .map_err(|e| failure(&e))?;
        #[derive(Serialize)]
        struct Out {
            folder: String,
            folders: Vec<String>,
            notes: Vec<NoteOut>,
            other_files: Vec<String>,
        }
        let mut out = Out {
            folder: listing.relative_path,
            folders: Vec::new(),
            notes: Vec::new(),
            other_files: Vec::new(),
        };
        for entry in listing.entries {
            match (entry.kind, entry.note) {
                (crate::models::FolderEntryKind::Folder, _) => {
                    out.folders.push(entry.relative_path)
                }
                (crate::models::FolderEntryKind::Note, Some(note)) => out.notes.push(note.into()),
                _ => out.other_files.push(entry.relative_path),
            }
        }
        reply(Ok(out))
    }

    #[tool(
        name = "list_tasks",
        description = "List tasks, open ones unless statuses say otherwise, optionally only those linked to a note or a repository.",
        annotations(read_only_hint = true)
    )]
    async fn list_tasks(&self, Parameters(args): Parameters<ListTasksArgs>) -> Reply {
        let statuses = args
            .statuses
            .map(|s| s.into_iter().map(TaskStatus::from).collect())
            .unwrap_or_else(|| vec![TaskStatus::Todo, TaskStatus::InProgress]);
        let filter = TaskFilter {
            statuses: Some(statuses),
            to_sort: args.to_sort,
            note_id: args.note_id,
            repository_id: args.repository_id,
        };
        let tasks = self
            .server
            .tasks
            .list(filter)
            .await
            .map_err(|e| failure(&e))?;
        let names = self.repository_names().await;
        reply(Ok(tasks_out(tasks, &names)))
    }

    #[tool(
        name = "get_task",
        description = "Read one task with its version, linked note, and repository.",
        annotations(read_only_hint = true)
    )]
    async fn get_task(&self, Parameters(args): Parameters<TaskArgs>) -> Reply {
        let task = self
            .server
            .tasks
            .get(&args.task_id)
            .await
            .map_err(|e| failure(&e))?;
        let names = self.repository_names().await;
        reply(Ok(task_out(task, &names)))
    }

    #[tool(
        name = "get_today",
        description = "The user's Today: open tasks overdue, due, or planned for today or earlier; tasks completed today; tasks still to sort; and the repositories in today's work with their state.",
        annotations(read_only_hint = true)
    )]
    async fn get_today(&self) -> Reply {
        let today = self.server.tasks.today().await.map_err(|e| failure(&e))?;
        let repositories = self.server.repositories.list().await.unwrap_or_default();
        let names = repositories
            .iter()
            .map(|r| (r.id.clone(), r.name.clone()))
            .collect();
        #[derive(Serialize)]
        struct Out {
            /// Today's date on the user's Mac, `YYYY-MM-DD`.
            date: String,
            open: Vec<TaskOut>,
            completed_today: Vec<TaskOut>,
            to_sort: Vec<TaskOut>,
            repositories: Vec<RepositoryOut>,
        }
        reply(Ok(Out {
            date: today.date,
            open: tasks_out(today.open, &names),
            completed_today: tasks_out(today.completed, &names),
            to_sort: tasks_out(today.to_sort, &names),
            repositories: repositories
                .into_iter()
                .filter(|r| today.repository_ids.contains(&r.id))
                .map(RepositoryOut::from)
                .collect(),
        }))
    }

    #[tool(
        name = "list_repositories",
        description = "The Git repositories Brainiac tracks, with the branch, changed files, and ahead/behind it last observed.",
        annotations(read_only_hint = true)
    )]
    async fn list_repositories(&self) -> Reply {
        let repositories = self.server.repositories.list().await;
        reply(repositories.map(|rs| rs.into_iter().map(RepositoryOut::from).collect::<Vec<_>>()))
    }

    #[tool(
        name = "repository_for_path",
        description = "The registered repository containing a folder or file, such as your working directory. Use its ID to find or link the repository's notes and tasks.",
        annotations(read_only_hint = true)
    )]
    async fn repository_for_path(&self, Parameters(args): Parameters<PathArgs>) -> Reply {
        let path = std::path::Path::new(&args.path);
        if !path.is_absolute() {
            return Err("VALIDATION: Give an absolute path.".into());
        }
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let repositories = self
            .server
            .repositories
            .list()
            .await
            .map_err(|e| failure(&e))?;
        // The innermost repository wins: a nested repository inside another.
        let found = repositories
            .into_iter()
            .filter_map(|r| {
                // Resolved like the path, so a symlinked folder (`/var` is
                // `/private/var`) matches either way.
                let depth = [&r.canonical_root, &r.display_path]
                    .iter()
                    .map(std::path::Path::new)
                    .flat_map(|root| [root.to_path_buf(), root.canonicalize().unwrap_or_default()])
                    .filter(|root| !root.as_os_str().is_empty() && path.starts_with(root))
                    .map(|root| root.components().count())
                    .max()?;
                Some((depth, r))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, r)| RepositoryOut::from(r));
        match found {
            Some(r) => reply(Ok(r)),
            None => Err("NOT_FOUND: No repository registered in Brainiac contains that path. The user can add it in Brainiac (File → Add Repository or Workspace…).".into()),
        }
    }

    #[tool(
        name = "get_repository_notes",
        description = "A repository's linked notes, its open tasks, and notes that mention it without being linked.",
        annotations(read_only_hint = true)
    )]
    async fn get_repository_notes(&self, Parameters(args): Parameters<RepositoryArgs>) -> Reply {
        let notes = self
            .server
            .notes
            .repository_notes(&args.repository_id)
            .await
            .map_err(|e| failure(&e))?;
        let names = self.repository_names().await;
        #[derive(Serialize)]
        struct Out {
            notes: Vec<NoteOut>,
            open_tasks: Vec<TaskOut>,
            /// Mention the repository by name but are not linked to it.
            suggested_notes: Vec<NoteOut>,
        }
        reply(Ok(Out {
            notes: notes.notes.into_iter().map(NoteOut::from).collect(),
            open_tasks: tasks_out(notes.tasks, &names),
            suggested_notes: notes.suggested.into_iter().map(NoteOut::from).collect(),
        }))
    }
}
