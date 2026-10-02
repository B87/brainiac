//! Tasks and Today (SPEC.md, section 6). Every write names the version it
//! read and fails with `CONFLICT` when the task changed meanwhile, so two
//! edits from two places never overwrite each other silently.

use std::sync::Arc;

use chrono::{NaiveDate, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::{enum_name, parse_enum};
use crate::models::{
    now_rfc3339, AppError, AppResult, ErrorCode, Task, TaskChangedEvent, TaskFields, TaskFilter,
    TaskNote, TaskStatus, TodayView, UpdateTaskRequest,
};
use crate::notes::{KnowledgeEvent, NoteService};

const TITLE_MAX: usize = 300;
const DESCRIPTION_MAX: usize = 2000;

const TASK_COLUMNS: &str =
    "t.id, t.title, t.description, t.status, t.triaged_at, t.planned_date, t.due_date,
    t.linked_note_id, n.title, n.relative_path,
    n.missing_at IS NOT NULL OR COALESCE((SELECT v.active FROM vaults v WHERE v.id = n.vault_id), 0) = 0,
    t.linked_repository_id, t.created_at,
    t.updated_at, t.completed_at, t.version";
const TASK_FROM: &str = "tasks t LEFT JOIN notes n ON n.id = t.linked_note_id";

fn row_to_task(r: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status: TaskStatus = parse_enum(r.get(3)?)?;
    let triaged: Option<String> = r.get(4)?;
    let planned: Option<String> = r.get(5)?;
    let due: Option<String> = r.get(6)?;
    let note_id: Option<String> = r.get(7)?;
    let note = match note_id {
        Some(id) => Some(TaskNote {
            id,
            title: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
            relative_path: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
            missing: r.get::<_, Option<bool>>(10)?.unwrap_or(true),
        }),
        None => None,
    };
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        description: r.get(2)?,
        to_sort: status.is_open() && triaged.is_none() && planned.is_none() && due.is_none(),
        status,
        planned_date: planned,
        due_date: due,
        note,
        repository_id: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
        completed_at: r.get(14)?,
        version: r.get(15)?,
    })
}

fn query_tasks(
    conn: &Connection,
    filter_sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<Task>> {
    let sql = format!("SELECT {TASK_COLUMNS} FROM {TASK_FROM} WHERE {filter_sql}");
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(args, row_to_task)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Open tasks first (in progress, then to do), by deadline, plan, and creation;
/// then finished ones, newest first.
const LIST_ORDER: &str = "ORDER BY t.status NOT IN ('todo', 'in_progress'),
    t.status <> 'in_progress', t.due_date IS NULL, t.due_date, t.planned_date IS NULL, t.planned_date,
    COALESCE(t.completed_at, t.updated_at) DESC, t.created_at DESC";

pub fn get(conn: &Connection, id: &str) -> AppResult<Option<Task>> {
    Ok(query_tasks(conn, "t.id = ?1", &[&id])?.into_iter().next())
}

/// Tasks linked to a note, for its context panel.
pub fn for_note(conn: &Connection, note_id: &str) -> AppResult<Vec<Task>> {
    query_tasks(
        conn,
        &format!("t.linked_note_id = ?1 {LIST_ORDER}"),
        &[&note_id],
    )
}

/// Open tasks linked to a repository, for its Notes tab.
pub fn open_for_repository(conn: &Connection, repository_id: &str) -> AppResult<Vec<Task>> {
    query_tasks(
        conn,
        &format!(
            "t.linked_repository_id = ?1 AND t.status IN ('todo', 'in_progress') {LIST_ORDER}"
        ),
        &[&repository_id],
    )
}

pub fn list(conn: &Connection, filter: &TaskFilter) -> AppResult<Vec<Task>> {
    let mut clauses: Vec<String> = vec!["1 = 1".into()];
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(statuses) = &filter.statuses {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let names: Vec<String> = statuses
            .iter()
            .map(|s| enum_name(*s))
            .collect::<AppResult<_>>()?;
        let marks: Vec<String> = names
            .iter()
            .map(|n| {
                args.push(Box::new(n.clone()));
                format!("?{}", args.len())
            })
            .collect();
        clauses.push(format!("t.status IN ({})", marks.join(", ")));
    }
    match filter.to_sort {
        Some(true) => clauses.push(
            "t.status IN ('todo', 'in_progress') AND t.triaged_at IS NULL AND t.planned_date IS NULL AND t.due_date IS NULL".into(),
        ),
        Some(false) => clauses.push(
            "NOT (t.status IN ('todo', 'in_progress') AND t.triaged_at IS NULL AND t.planned_date IS NULL AND t.due_date IS NULL)".into(),
        ),
        None => {}
    }
    if let Some(note) = &filter.note_id {
        args.push(Box::new(note.clone()));
        clauses.push(format!("t.linked_note_id = ?{}", args.len()));
    }
    if let Some(repo) = &filter.repository_id {
        args.push(Box::new(repo.clone()));
        clauses.push(format!("t.linked_repository_id = ?{}", args.len()));
    }
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
    query_tasks(
        conn,
        &format!("{} {LIST_ORDER}", clauses.join(" AND ")),
        &refs,
    )
}

/// Today for the local date `today`, whose day runs from `start` to `end`
/// (RFC 3339 UTC instants), so "completed today" follows the Mac's clock.
pub fn today(conn: &Connection, today: NaiveDate, start: &str, end: &str) -> AppResult<TodayView> {
    let date = today.format("%Y-%m-%d").to_string();
    let open = query_tasks(
        conn,
        "t.status IN ('todo', 'in_progress') AND (t.due_date <= ?1 OR t.planned_date <= ?1)
         ORDER BY CASE WHEN t.due_date < ?1 THEN 0 WHEN t.due_date = ?1 THEN 1 ELSE 2 END,
           t.status <> 'in_progress', COALESCE(t.due_date, t.planned_date), t.created_at",
        &[&date],
    )?;
    let completed = query_tasks(
        conn,
        "t.status = 'done' AND t.completed_at >= ?1 AND t.completed_at < ?2 ORDER BY t.completed_at DESC",
        &[&start, &end],
    )?;
    let to_sort = query_tasks(
        conn,
        "t.status IN ('todo', 'in_progress') AND t.triaged_at IS NULL AND t.planned_date IS NULL
           AND t.due_date IS NULL ORDER BY t.created_at DESC",
        &[],
    )?;
    let mut repository_ids: Vec<String> = Vec::new();
    {
        let mut by_note = conn.prepare_cached(
            "SELECT l.repository_id FROM note_repository_links l JOIN repositories r ON r.id = l.repository_id
             WHERE l.note_id = ?1 ORDER BY l.repository_name COLLATE NOCASE",
        )?;
        let mut registered =
            conn.prepare_cached("SELECT EXISTS (SELECT 1 FROM repositories WHERE id = ?1)")?;
        for task in open.iter().chain(completed.iter()) {
            if let Some(r) = &task.repository_id {
                if registered.query_row([r], |row| row.get::<_, bool>(0))?
                    && !repository_ids.contains(r)
                {
                    repository_ids.push(r.clone());
                }
            }
            if let Some(note) = &task.note {
                let rows: Vec<String> = by_note
                    .query_map([&note.id], |row| row.get(0))?
                    .collect::<Result<_, _>>()?;
                for r in rows {
                    if !repository_ids.contains(&r) {
                        repository_ids.push(r);
                    }
                }
            }
        }
    }
    Ok(TodayView {
        date,
        open,
        completed,
        to_sort,
        repository_ids,
    })
}

fn check_date(label: &str, date: &Option<String>) -> AppResult<()> {
    if let Some(d) = date {
        let parsed = NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
        if parsed.map(|p| p.format("%Y-%m-%d").to_string()).as_deref() != Some(d.as_str()) {
            return Err(AppError::validation(format!(
                "The {label} must be a calendar date like 2026-10-03."
            )));
        }
    }
    Ok(())
}

/// Validate and normalize what the editor sent.
fn clean(fields: &TaskFields) -> AppResult<TaskFields> {
    let title: String = fields
        .title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        return Err(AppError::validation("A task needs a title."));
    }
    if title.chars().count() > TITLE_MAX {
        return Err(AppError::validation(format!(
            "A task's title is at most {TITLE_MAX} characters. Longer material belongs in a linked note."
        )));
    }
    let description = fields.description.trim().to_string();
    if description.chars().count() > DESCRIPTION_MAX {
        return Err(AppError::validation(format!(
            "A task's description is at most {DESCRIPTION_MAX} characters. Longer material belongs in a linked note."
        )));
    }
    check_date("planned date", &fields.planned_date)?;
    check_date("deadline", &fields.due_date)?;
    Ok(TaskFields {
        title,
        description,
        ..fields.clone()
    })
}

/// Check that a newly linked note or repository exists. Links the task
/// already had are kept even when their target is gone.
fn check_links(conn: &Connection, fields: &TaskFields, current: Option<&Task>) -> AppResult<()> {
    if let Some(note) = &fields.note_id {
        let unchanged = current
            .and_then(|t| t.note.as_ref())
            .is_some_and(|n| &n.id == note);
        if !unchanged {
            let live: Option<bool> = conn
                .query_row(
                    "SELECT missing_at IS NULL FROM notes WHERE id = ?1",
                    [note],
                    |r| r.get(0),
                )
                .optional()?;
            if live != Some(true) {
                return Err(AppError::not_found("That note does not exist."));
            }
        }
    }
    if let Some(repo) = &fields.repository_id {
        let unchanged = current.and_then(|t| t.repository_id.as_ref()) == Some(repo);
        if !unchanged && crate::db::get_repository(conn, repo)?.is_none() {
            return Err(AppError::not_found("That repository is not registered."));
        }
    }
    Ok(())
}

pub struct TaskService {
    notes: Arc<NoteService>,
}

impl TaskService {
    pub fn new(notes: Arc<NoteService>) -> Self {
        TaskService { notes }
    }

    fn emit(&self, task_id: &str, version: Option<i64>) {
        self.notes
            .emit(KnowledgeEvent::TaskChanged(TaskChangedEvent {
                task_id: task_id.to_string(),
                version,
            }));
    }

    /// Write the linked note's `brainiac_id`, which now carries context.
    async fn note_linked(&self, before: Option<&Task>, after: &Task) {
        if let Some(note) = &after.note {
            if before.and_then(|b| b.note.as_ref()).map(|n| &n.id) != Some(&note.id) {
                if let Err(e) = self.notes.ensure_embedded_id(&note.id).await {
                    tracing::warn!(error = %e, "could not write brainiac_id into the note");
                }
            }
        }
        for note in [before.and_then(|b| b.note.as_ref()), after.note.as_ref()]
            .into_iter()
            .flatten()
        {
            self.notes.announce_context(&note.id).await;
        }
    }

    pub async fn list(&self, filter: TaskFilter) -> AppResult<Vec<Task>> {
        self.notes
            .core()
            .call(move |conn| list(conn, &filter))
            .await
    }

    pub async fn get(&self, id: &str) -> AppResult<Task> {
        let id = id.to_string();
        self.notes
            .core()
            .call(move |conn| get(conn, &id))
            .await?
            .ok_or_else(|| AppError::not_found("That task does not exist."))
    }

    /// Today for the Mac's current date (SPEC.md, Tasks and Today).
    pub async fn today(&self) -> AppResult<TodayView> {
        let now = chrono::Local::now();
        self.today_on(now.date_naive()).await
    }

    /// Today as it is on a given local date.
    pub async fn today_on(&self, date: NaiveDate) -> AppResult<TodayView> {
        let bound = |d: NaiveDate| {
            let midnight = d.and_hms_opt(0, 0, 0).expect("midnight");
            chrono::Local
                .from_local_datetime(&midnight)
                .earliest()
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_else(|| midnight.and_utc())
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        let start = bound(date);
        let end = bound(date.succ_opt().unwrap_or(date));
        self.notes
            .core()
            .call(move |conn| today(conn, date, &start, &end))
            .await
    }

    pub async fn create(&self, fields: TaskFields) -> AppResult<Task> {
        let fields = clean(&fields)?;
        let id = uuid::Uuid::new_v4().to_string();
        let id2 = id.clone();
        let task = self
            .notes
            .core()
            .call(move |conn| {
                let tx = conn.transaction()?;
                check_links(&tx, &fields, None)?;
                let now = now_rfc3339();
                let has_date = fields.planned_date.is_some() || fields.due_date.is_some();
                let triaged = (fields.sorted || has_date).then(|| now.clone());
                let completed = (fields.status == TaskStatus::Done).then(|| now.clone());
                tx.execute(
                    "INSERT INTO tasks (id, title, description, status, triaged_at, planned_date, due_date,
                       linked_note_id, linked_repository_id, created_at, updated_at, completed_at, version)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11, 1)",
                    params![
                        id2,
                        fields.title,
                        fields.description,
                        enum_name(fields.status)?,
                        triaged,
                        fields.planned_date,
                        fields.due_date,
                        fields.note_id,
                        fields.repository_id,
                        now,
                        completed
                    ],
                )?;
                let task = get(&tx, &id2)?.expect("inserted task");
                tx.commit()?;
                Ok(task)
            })
            .await?;
        self.emit(&id, Some(task.version));
        self.note_linked(None, &task).await;
        Ok(task)
    }

    /// Replace a task's fields if it is still at `expected_version`.
    pub async fn update(&self, request: UpdateTaskRequest) -> AppResult<Task> {
        let fields = clean(&request.fields)?;
        let (id, expected) = (request.task_id.clone(), request.expected_version);
        let (before, after) = self
            .notes
            .core()
            .call(move |conn| {
                let tx = conn.transaction()?;
                let current = get(&tx, &id)?.ok_or_else(|| AppError::not_found("That task does not exist."))?;
                if current.version != expected {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "This task was changed elsewhere. Showing its current version.",
                    ));
                }
                check_links(&tx, &fields, Some(&current))?;
                let now = now_rfc3339();
                let has_date = fields.planned_date.is_some() || fields.due_date.is_some();
                let triaged_at: Option<String> = tx.query_row(
                    "SELECT triaged_at FROM tasks WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )?;
                let triaged = if fields.sorted || has_date {
                    Some(triaged_at.unwrap_or_else(|| now.clone()))
                } else {
                    None
                };
                // Completing records when; reopening or cancelling clears it.
                let completed = match (current.status, fields.status) {
                    (TaskStatus::Done, TaskStatus::Done) => current.completed_at.clone(),
                    (_, TaskStatus::Done) => Some(now.clone()),
                    _ => None,
                };
                let changed = tx.execute(
                    "UPDATE tasks SET title = ?3, description = ?4, status = ?5, triaged_at = ?6,
                       planned_date = ?7, due_date = ?8, linked_note_id = ?9, linked_repository_id = ?10,
                       updated_at = ?11, completed_at = ?12, version = version + 1
                     WHERE id = ?1 AND version = ?2",
                    params![
                        id,
                        expected,
                        fields.title,
                        fields.description,
                        enum_name(fields.status)?,
                        triaged,
                        fields.planned_date,
                        fields.due_date,
                        fields.note_id,
                        fields.repository_id,
                        now,
                        completed
                    ],
                )?;
                if changed == 0 {
                    return Err(AppError::new(ErrorCode::Conflict, "This task was changed elsewhere."));
                }
                let after = get(&tx, &id)?.expect("updated task");
                tx.commit()?;
                Ok((current, after))
            })
            .await?;
        self.emit(&after.id, Some(after.version));
        self.note_linked(Some(&before), &after).await;
        Ok(after)
    }

    pub async fn delete(&self, id: &str, expected_version: i64) -> AppResult<()> {
        let id2 = id.to_string();
        let before = self
            .notes
            .core()
            .call(move |conn| {
                let current = get(conn, &id2)?
                    .ok_or_else(|| AppError::not_found("That task does not exist."))?;
                if current.version != expected_version {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "This task was changed elsewhere. Showing its current version.",
                    ));
                }
                conn.execute(
                    "DELETE FROM tasks WHERE id = ?1 AND version = ?2",
                    params![id2, expected_version],
                )?;
                Ok(current)
            })
            .await?;
        self.emit(id, None);
        if let Some(note) = &before.note {
            self.notes.announce_context(&note.id).await;
        }
        Ok(())
    }
}
