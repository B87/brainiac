//! The `agent_runs` rows of `history.db` (migrations `history/0004` and
//! `0005`, which adds the kind): a
//! run's immutable start and settings, the controller's last confirmed
//! projection, the mirrored cursor, the collected result, and pending
//! client actions. Never the token or key.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{activity_of, outcome_of, permission_of, phase_of};
use crate::agents::controller::protocol::RunStatus;
use crate::models::{
    now_rfc3339, AgentKind, AgentPayment, AgentProvider, AgentRun, AppError, AppResult,
    LeftOutFile, RunActivity, RunCollection, RunOutcome, RunPermissionRequest, RunPermissions,
    RunPhase,
};

/// What a start knows before the controller answers.
pub struct NewRun {
    pub id: String,
    pub repository_id: String,
    pub repository_name: String,
    pub title: String,
    pub start_commit: String,
    pub start_subject: String,
    pub profile_id: String,
    pub agent: AgentKind,
    pub provider: AgentProvider,
    pub payment: AgentPayment,
    pub credential_source: String,
    pub host_id: String,
    pub host_name: String,
    pub engine_socket: String,
    pub engine_name: String,
    pub image_name: String,
    pub image_id: String,
    pub permissions: RunPermissions,
    pub time_limit_minutes: u32,
    pub cpus: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    pub model: String,
    /// An explain run (SPEC.md, section 14): left out of Runs, its badge,
    /// retention, and collection.
    pub explain: bool,
}

/// One row, as stored.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: String,
    pub repository_id: String,
    pub repository_name: String,
    pub title: String,
    pub start_commit: String,
    pub start_subject: String,
    pub profile_id: String,
    pub agent: AgentKind,
    pub provider: AgentProvider,
    pub payment: AgentPayment,
    pub credential_source: String,
    pub host_id: String,
    pub host_name: String,
    pub engine_socket: String,
    pub engine_name: String,
    pub image_name: String,
    pub image_id: String,
    pub permissions: RunPermissions,
    pub time_limit_minutes: u32,
    pub cpus: u32,
    pub memory_mib: u32,
    pub workspace_gib: u32,
    pub model: String,
    pub model_used: Option<String>,
    pub attempt: u32,
    pub phase: RunPhase,
    pub activity: RunActivity,
    pub turn: u32,
    pub outcome: Option<RunOutcome>,
    pub stop_confirmed: bool,
    pub kept: bool,
    pub session_id: Option<String>,
    pub accepted_at: Option<String>,
    pub deadline_at: Option<String>,
    pub ended_at: Option<String>,
    pub expired_asleep: bool,
    pub error: Option<String>,
    pub pending_permissions: Vec<RunPermissionRequest>,
    pub cursor: u64,
    pub reported_at: Option<String>,
    pub cancel_requested: bool,
    pub collection: RunCollection,
    pub collection_error: Option<String>,
    pub result_commit: Option<String>,
    pub changed_files: Option<u32>,
    pub left_out: Vec<LeftOutFile>,
    pub left_out_more: u32,
    pub snapshot_accepted: bool,
    pub cleanup_pending: Option<String>,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
    pub explain: bool,
}

impl RunRow {
    pub fn new(new: NewRun) -> Self {
        let now = now_rfc3339();
        RunRow {
            id: new.id,
            repository_id: new.repository_id,
            repository_name: new.repository_name,
            title: new.title,
            start_commit: new.start_commit,
            start_subject: new.start_subject,
            profile_id: new.profile_id,
            agent: new.agent,
            provider: new.provider,
            payment: new.payment,
            credential_source: new.credential_source,
            host_id: new.host_id,
            host_name: new.host_name,
            engine_socket: new.engine_socket,
            engine_name: new.engine_name,
            image_name: new.image_name,
            image_id: new.image_id,
            permissions: new.permissions,
            time_limit_minutes: new.time_limit_minutes,
            cpus: new.cpus,
            memory_mib: new.memory_mib,
            workspace_gib: new.workspace_gib,
            model: new.model,
            model_used: None,
            attempt: 1,
            phase: RunPhase::Preparing,
            activity: RunActivity::Preparing,
            turn: 0,
            outcome: None,
            stop_confirmed: false,
            kept: true,
            session_id: None,
            accepted_at: None,
            deadline_at: None,
            ended_at: None,
            expired_asleep: false,
            error: None,
            pending_permissions: Vec::new(),
            cursor: 0,
            reported_at: None,
            cancel_requested: false,
            collection: RunCollection::None,
            collection_error: None,
            result_commit: None,
            changed_files: None,
            left_out: Vec::new(),
            left_out_more: 0,
            snapshot_accepted: false,
            cleanup_pending: None,
            version: 1,
            created_at: now.clone(),
            updated_at: now,
            explain: new.explain,
        }
    }

    /// The row as the window sees it.
    pub fn into_run(self, connected: bool) -> AgentRun {
        AgentRun {
            id: self.id,
            repository_id: self.repository_id,
            repository_name: self.repository_name,
            title: self.title,
            start_commit: self.start_commit,
            start_subject: self.start_subject,
            profile_id: self.profile_id,
            agent: self.agent,
            provider: self.provider,
            payment: self.payment,
            credential_source: self.credential_source,
            host_id: self.host_id,
            host_name: self.host_name,
            engine_name: self.engine_name,
            image_name: self.image_name,
            permissions: self.permissions,
            time_limit_minutes: self.time_limit_minutes,
            cpus: self.cpus,
            memory_mib: self.memory_mib,
            workspace_gib: self.workspace_gib,
            model: self.model,
            model_used: self.model_used,
            phase: self.phase,
            activity: self.activity,
            turn: self.turn,
            outcome: self.outcome,
            stop_confirmed: self.stop_confirmed,
            kept: self.kept,
            accepted_at: self.accepted_at,
            deadline_at: self.deadline_at,
            ended_at: self.ended_at,
            expired_asleep: self.expired_asleep,
            error: self.error,
            pending_permissions: self.pending_permissions,
            // An ended run needs no controller.
            connected: connected || self.phase == RunPhase::Ended,
            reported_at: self.reported_at,
            cancel_requested: self.cancel_requested,
            collection: self.collection,
            collection_error: self.collection_error,
            result_commit: self.result_commit,
            changed_files: self.changed_files,
            left_out: self.left_out,
            left_out_more: self.left_out_more,
            snapshot_accepted: self.snapshot_accepted,
            cleanup_pending: self.cleanup_pending,
            explain: self.explain,
            cursor: self.cursor,
            created_at: self.created_at,
            updated_at: self.updated_at,
            version: self.version,
            starting: None,
        }
    }

    /// The run still needs the controller: it is live, a cancel waits, or
    /// its work is to be collected on its own.
    pub fn needs_sync(&self) -> bool {
        self.phase != RunPhase::Ended
            || self.cancel_requested
            || (self.kept
                && self.stop_confirmed
                && self.collection == RunCollection::None
                && self.outcome != Some(RunOutcome::Interrupted))
    }

    /// Work waits for the user: uncollected, left-out files not accepted, a
    /// failed collection, or a cleanup still to do.
    pub fn waits_for_decision(&self) -> bool {
        self.kept
            || self.cleanup_pending.is_some()
            || self.collection == RunCollection::Failed
            || (matches!(
                self.collection,
                RunCollection::Ready | RunCollection::NoChanges
            ) && !self.snapshot_accepted)
    }

    /// The start and result to review, once collected.
    pub fn reviewable(&self) -> AppResult<(String, String)> {
        match (&self.collection, &self.result_commit) {
            (RunCollection::Ready | RunCollection::NoChanges, Some(result)) => {
                Ok((self.start_commit.clone(), result.clone()))
            }
            _ => Err(AppError::new(
                crate::models::ErrorCode::Conflict,
                "The run's work has not been collected yet.",
            )),
        }
    }
}

/// The controller's status as the row keeps it.
pub struct Projection {
    pub phase: RunPhase,
    pub activity: RunActivity,
    pub turn: u32,
    pub outcome: Option<RunOutcome>,
    pub stop_confirmed: bool,
    pub kept: bool,
    pub session_id: Option<String>,
    pub model_used: Option<String>,
    pub accepted_at: String,
    pub deadline_at: String,
    pub ended_at: Option<String>,
    pub expired_asleep: bool,
    pub error: Option<String>,
    pub pending_permissions: Vec<RunPermissionRequest>,
    pub attempt: u32,
}

impl Projection {
    pub fn from_status(status: &RunStatus) -> Self {
        Projection {
            phase: phase_of(status.phase),
            activity: activity_of(status.activity),
            turn: status.turn,
            outcome: status.outcome.map(outcome_of),
            stop_confirmed: status.stop_confirmed,
            kept: status.kept,
            session_id: status.session_id.clone(),
            model_used: status.model.clone(),
            accepted_at: status.accepted_at.clone(),
            deadline_at: status.deadline_at.clone(),
            ended_at: status.ended_at.clone(),
            expired_asleep: status.expired_asleep,
            error: status.error.clone(),
            pending_permissions: status
                .permissions
                .iter()
                .filter_map(permission_of)
                .collect(),
            attempt: status.attempt,
        }
    }
}

/// What a collection produced.
pub struct Collected {
    pub collection: RunCollection,
    pub result_commit: String,
    pub changed_files: u32,
    pub left_out: Vec<LeftOutFile>,
    pub left_out_more: u32,
    pub accepted: bool,
}

const COLUMNS: &str = "id, repository_id, repository_name, title, start_commit, start_subject,
    profile_id, payment, credential_source, engine_socket, engine_name, image_name, image_id,
    permissions, time_limit_minutes, cpus, memory_mib, workspace_gib, attempt, phase, activity,
    turn, outcome, stop_confirmed, kept, session_id, accepted_at, deadline_at, ended_at,
    expired_asleep, error, pending_permissions, cursor, reported_at, cancel_requested,
    collection, collection_error, result_commit, changed_files, left_out, left_out_more,
    snapshot_accepted, cleanup_pending, version, created_at, updated_at, model, model_used,
    host_id, host_name, agent, provider, kind";

fn word<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_json<T: serde::de::DeserializeOwned + Default>(text: String) -> T {
    serde_json::from_str(&text).unwrap_or_default()
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        repository_id: r.get(1)?,
        repository_name: r.get(2)?,
        title: r.get(3)?,
        start_commit: r.get(4)?,
        start_subject: r.get(5)?,
        profile_id: r.get(6)?,
        payment: parse(r.get(7)?)?,
        credential_source: r.get(8)?,
        engine_socket: r.get(9)?,
        engine_name: r.get(10)?,
        image_name: r.get(11)?,
        image_id: r.get(12)?,
        permissions: parse(r.get(13)?)?,
        time_limit_minutes: r.get(14)?,
        cpus: r.get(15)?,
        memory_mib: r.get(16)?,
        workspace_gib: r.get(17)?,
        attempt: r.get(18)?,
        phase: parse(r.get(19)?)?,
        activity: parse(r.get(20)?)?,
        turn: r.get(21)?,
        outcome: r.get::<_, Option<String>>(22)?.map(parse).transpose()?,
        stop_confirmed: r.get(23)?,
        kept: r.get(24)?,
        session_id: r.get(25)?,
        accepted_at: r.get(26)?,
        deadline_at: r.get(27)?,
        ended_at: r.get(28)?,
        expired_asleep: r.get(29)?,
        error: r.get(30)?,
        pending_permissions: parse_json(r.get(31)?),
        cursor: r.get::<_, i64>(32)?.max(0) as u64,
        reported_at: r.get(33)?,
        cancel_requested: r.get(34)?,
        collection: parse(r.get(35)?)?,
        collection_error: r.get(36)?,
        result_commit: r.get(37)?,
        changed_files: r.get(38)?,
        left_out: parse_json(r.get(39)?),
        left_out_more: r.get(40)?,
        snapshot_accepted: r.get(41)?,
        cleanup_pending: r.get(42)?,
        version: r.get(43)?,
        created_at: r.get(44)?,
        updated_at: r.get(45)?,
        model: r.get(46)?,
        model_used: r.get(47)?,
        host_id: r.get(48)?,
        host_name: r.get(49)?,
        agent: parse(r.get(50)?)?,
        provider: parse(r.get(51)?)?,
        explain: r.get::<_, String>(52)? == "explain",
    })
}

pub fn list(conn: &mut Connection) -> AppResult<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM agent_runs ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get(conn: &mut Connection, id: &str) -> AppResult<Option<RunRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM agent_runs WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

pub fn insert(conn: &mut Connection, row: &RunRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO agent_runs (id, repository_id, repository_name, title, start_commit,
           start_subject, profile_id, payment, credential_source, host_id, host_name,
           engine_socket, engine_name, image_name, image_id, permissions, time_limit_minutes,
           cpus, memory_mib, workspace_gib, created_at, updated_at, model, agent, provider, kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
           ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
        params![
            row.id,
            row.repository_id,
            row.repository_name,
            row.title,
            row.start_commit,
            row.start_subject,
            row.profile_id,
            row.payment.as_str(),
            row.credential_source,
            row.host_id,
            row.host_name,
            row.engine_socket,
            row.engine_name,
            row.image_name,
            row.image_id,
            row.permissions.as_str(),
            row.time_limit_minutes,
            row.cpus,
            row.memory_mib,
            row.workspace_gib,
            row.created_at,
            row.updated_at,
            row.model,
            row.agent.as_str(),
            row.provider.as_str(),
            if row.explain { "explain" } else { "run" },
        ],
    )?;
    Ok(())
}

/// Apply the controller's status; `true` when anything changed.
pub fn apply(conn: &mut Connection, id: &str, p: &Projection) -> AppResult<bool> {
    let permissions = serde_json::to_string(&p.pending_permissions).unwrap_or_default();
    let changed = conn.execute(
        "UPDATE agent_runs SET phase = ?2, activity = ?3, turn = ?4, outcome = ?5,
           stop_confirmed = ?6, kept = ?7, session_id = ?8, accepted_at = ?9, deadline_at = ?10,
           ended_at = ?11, expired_asleep = ?12, error = ?13, pending_permissions = ?14,
           reported_at = ?15, attempt = ?16, model_used = ?17, version = version + 1,
           updated_at = ?15
         WHERE id = ?1 AND NOT (phase = ?2 AND activity = ?3 AND turn = ?4
           AND outcome IS ?5 AND stop_confirmed = ?6 AND kept = ?7 AND session_id IS ?8
           AND accepted_at IS ?9 AND deadline_at IS ?10 AND ended_at IS ?11
           AND expired_asleep = ?12 AND error IS ?13 AND pending_permissions = ?14
           AND model_used IS ?17)",
        params![
            id,
            word(&p.phase),
            word(&p.activity),
            p.turn,
            p.outcome.as_ref().map(word),
            p.stop_confirmed,
            p.kept,
            p.session_id,
            p.accepted_at,
            p.deadline_at,
            p.ended_at,
            p.expired_asleep,
            p.error,
            permissions,
            now_rfc3339(),
            p.attempt,
            p.model_used,
        ],
    )?;
    if changed == 0 {
        // Nothing changed but the controller answered: remember when.
        conn.execute(
            "UPDATE agent_runs SET reported_at = ?2 WHERE id = ?1",
            params![id, now_rfc3339()],
        )?;
    }
    Ok(changed > 0)
}

pub fn set_cursor(conn: &mut Connection, id: &str, cursor: u64) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET cursor = ?2, version = version + 1 WHERE id = ?1 AND cursor < ?2",
        params![id, cursor as i64],
    )?;
    Ok(())
}

pub fn set_cancel_requested(conn: &mut Connection, id: &str, requested: bool) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET cancel_requested = ?2, version = version + 1, updated_at = ?3
         WHERE id = ?1",
        params![id, requested, now_rfc3339()],
    )?;
    Ok(())
}

/// Cancel came before the controller was asked: nothing of the run exists
/// anywhere, and it ends as cancelled.
pub fn mark_cancelled_unstarted(conn: &mut Connection, id: &str) -> AppResult<()> {
    let now = now_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET phase = 'ended', activity = 'ended', outcome = 'cancelled',
           stop_confirmed = 1, kept = 0, cancel_requested = 0, ended_at = ?2,
           version = version + 1, updated_at = ?2
         WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

/// The controller refused the start: nothing of the run exists anywhere.
pub fn mark_refused(conn: &mut Connection, id: &str, message: &str) -> AppResult<()> {
    let now = now_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET phase = 'ended', activity = 'ended', outcome = 'failed',
           stop_confirmed = 1, kept = 0, ended_at = ?3, error = ?2, version = version + 1,
           updated_at = ?3
         WHERE id = ?1",
        params![id, message, now],
    )?;
    Ok(())
}

/// The controller does not know the run: the records of the controller
/// that ran it are gone, and so, most likely, is its container.
pub fn mark_lost(conn: &mut Connection, id: &str) -> AppResult<()> {
    let now = now_rfc3339();
    conn.execute(
        "UPDATE agent_runs SET phase = 'ended', activity = 'ended',
           outcome = COALESCE(outcome, 'interrupted'), stop_confirmed = 1, kept = 0,
           ended_at = COALESCE(ended_at, ?2), cancel_requested = 0,
           error = COALESCE(error, 'The run controller no longer knows this run; its container may be gone.'),
           version = version + 1, updated_at = ?2
         WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

pub fn set_collection(
    conn: &mut Connection,
    id: &str,
    state: RunCollection,
    error: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET collection = ?2, collection_error = ?3, version = version + 1,
           updated_at = ?4
         WHERE id = ?1",
        params![id, state.as_str(), error, now_rfc3339()],
    )?;
    Ok(())
}

pub fn set_collected(conn: &mut Connection, id: &str, c: &Collected) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET collection = ?2, collection_error = NULL, result_commit = ?3,
           changed_files = ?4, left_out = ?5, left_out_more = ?6, snapshot_accepted = ?7,
           version = version + 1, updated_at = ?8
         WHERE id = ?1",
        params![
            id,
            c.collection.as_str(),
            c.result_commit,
            c.changed_files,
            serde_json::to_string(&c.left_out).unwrap_or_default(),
            c.left_out_more,
            c.accepted,
            now_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn set_accepted(conn: &mut Connection, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET snapshot_accepted = 1, version = version + 1, updated_at = ?2
         WHERE id = ?1",
        params![id, now_rfc3339()],
    )?;
    Ok(())
}

/// After a cleanup attempt: gone, or pending with its reason.
pub fn set_cleanup(
    conn: &mut Connection,
    id: &str,
    removed: bool,
    pending: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE agent_runs SET kept = CASE WHEN ?2 THEN 0 ELSE kept END, cleanup_pending = ?3,
           version = version + 1, updated_at = ?4
         WHERE id = ?1",
        params![id, removed, pending, now_rfc3339()],
    )?;
    Ok(())
}

pub fn delete(conn: &mut Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM agent_runs WHERE id = ?1", [id])?;
    Ok(())
}
