//! Storage of explanations (docs/architecture.md, Explaining changes —
//! v0.6, Storage): the `explanations` rows of `history.db`, and in
//! `brainiac.db` each repository's or workspace's answer, the known
//! concepts, and Settings → Explanations (one value in `settings`).

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{
    now_rfc3339, AgentKind, AgentPayment, AgentProvider, AppError, AppResult, ConceptKind,
    ExplainAnswer, ExplainAnswerScope, ExplainCost, ExplainDepth, ExplainStep, ExplainSubject,
    Explanation, ExplanationSettings, ExplanationState, KnownConcept,
};

/// The settings key Settings → Explanations is saved under.
const SETTINGS_KEY: &str = "explanations";

/// One row of `explanations`.
#[derive(Debug, Clone)]
pub struct ExplanationRow {
    pub id: String,
    pub repository_id: String,
    pub repository_name: String,
    pub subject: ExplainSubject,
    pub title: String,
    pub base: String,
    pub tip: String,
    pub profile_id: String,
    pub agent: AgentKind,
    pub provider: AgentProvider,
    pub payment: AgentPayment,
    pub host_id: String,
    pub host_name: String,
    pub model: String,
    pub depth: ExplainDepth,
    pub questions: bool,
    pub time_limit_minutes: u32,
    pub state: ExplanationState,
    pub step: Option<ExplainStep>,
    pub error: Option<String>,
    pub errors: Vec<String>,
    pub run_id: Option<String>,
    pub explanation: Option<Explanation>,
    pub hidden: Vec<u32>,
    pub cost: Option<ExplainCost>,
    pub duration_secs: Option<u32>,
    pub created_at: String,
    pub ended_at: Option<String>,
    /// The stored JSON's length, for Settings.
    pub explanation_bytes: u64,
}

fn word<T: serde::Serialize>(value: &T) -> AppResult<String> {
    crate::db::enum_name(value)
}

fn parse<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    crate::db::parse_enum(text)
}

const COLUMNS: &str = "id, repository_id, repository_name, subject_kind, subject_ref, title,
    base, tip, profile_id, agent, provider, payment, host_id, host_name, model, depth,
    questions, time_limit_minutes, state, step, error, errors, run_id, explanation, hidden,
    cost_micros, currency, duration_secs, created_at, ended_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<ExplanationRow> {
    let explanation: Option<String> = r.get(23)?;
    let cost_micros: Option<i64> = r.get(25)?;
    let currency: Option<String> = r.get(26)?;
    Ok(ExplanationRow {
        id: r.get(0)?,
        repository_id: r.get(1)?,
        repository_name: r.get(2)?,
        subject: ExplainSubject {
            kind: parse(r.get(3)?)?,
            reference: r.get(4)?,
        },
        title: r.get(5)?,
        base: r.get(6)?,
        tip: r.get(7)?,
        profile_id: r.get(8)?,
        agent: parse(r.get(9)?)?,
        provider: parse(r.get(10)?)?,
        payment: parse(r.get(11)?)?,
        host_id: r.get(12)?,
        host_name: r.get(13)?,
        model: r.get(14)?,
        depth: parse(r.get(15)?)?,
        questions: r.get(16)?,
        time_limit_minutes: r.get(17)?,
        state: parse(r.get(18)?)?,
        step: r.get::<_, Option<String>>(19)?.map(parse).transpose()?,
        error: r.get(20)?,
        errors: serde_json::from_str(&r.get::<_, String>(21)?).unwrap_or_default(),
        run_id: r.get(22)?,
        explanation_bytes: explanation.as_ref().map_or(0, |e| e.len() as u64),
        explanation: explanation.and_then(|e| serde_json::from_str(&e).ok()),
        hidden: serde_json::from_str(&r.get::<_, String>(24)?).unwrap_or_default(),
        cost: cost_micros.map(|m| ExplainCost {
            micros: m.max(0) as u64,
            currency: currency.unwrap_or_else(|| "USD".into()),
        }),
        duration_secs: r.get(27)?,
        created_at: r.get(28)?,
        ended_at: r.get(29)?,
    })
}

pub fn get(conn: &mut Connection, id: &str) -> AppResult<Option<ExplanationRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM explanations WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

pub fn by_run(conn: &mut Connection, run_id: &str) -> AppResult<Option<ExplanationRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM explanations WHERE run_id = ?1"),
            [run_id],
            from_row,
        )
        .optional()?)
}

pub fn list(conn: &mut Connection) -> AppResult<Vec<ExplanationRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM explanations ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn for_subject(
    conn: &mut Connection,
    repository_id: &str,
    subject: &ExplainSubject,
) -> AppResult<Vec<ExplanationRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM explanations
         WHERE repository_id = ?1 AND subject_kind = ?2 AND subject_ref = ?3
         ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(
            params![repository_id, word(&subject.kind)?, subject.reference],
            from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The explanations of the same changes, whatever their subject: a branch
/// and a pull request with the same merge base and head share one
/// (SPEC.md, section 14, Pull requests).
pub fn for_range(
    conn: &mut Connection,
    repository_id: &str,
    base: &str,
    tip: &str,
) -> AppResult<Vec<ExplanationRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM explanations
         WHERE repository_id = ?1 AND tip = ?2 AND base = ?3
         ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![repository_id, tip, base], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The one of the same subject, profile, and depth.
pub fn find(
    conn: &mut Connection,
    repository_id: &str,
    subject: &ExplainSubject,
    profile_id: &str,
    depth: ExplainDepth,
) -> AppResult<Option<ExplanationRow>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM explanations
                 WHERE repository_id = ?1 AND subject_kind = ?2 AND subject_ref = ?3
                   AND profile_id = ?4 AND depth = ?5"
            ),
            params![
                repository_id,
                word(&subject.kind)?,
                subject.reference,
                profile_id,
                depth.as_str()
            ],
            from_row,
        )
        .optional()?)
}

pub fn insert(conn: &mut Connection, row: &ExplanationRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO explanations (id, repository_id, repository_name, subject_kind,
           subject_ref, title, base, tip, profile_id, agent, provider, payment, host_id,
           host_name, model, depth, questions, time_limit_minutes, state, step, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
           ?18, ?19, ?20, ?21)",
        params![
            row.id,
            row.repository_id,
            row.repository_name,
            word(&row.subject.kind)?,
            row.subject.reference,
            row.title,
            row.base,
            row.tip,
            row.profile_id,
            row.agent.as_str(),
            row.provider.as_str(),
            row.payment.as_str(),
            row.host_id,
            row.host_name,
            row.model,
            row.depth.as_str(),
            row.questions,
            row.time_limit_minutes,
            word(&row.state)?,
            row.step.map(|s| word(&s)).transpose()?,
            row.created_at,
        ],
    )?;
    Ok(())
}

pub fn set_run(conn: &mut Connection, id: &str, run_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE explanations SET run_id = ?2 WHERE id = ?1",
        params![id, run_id],
    )?;
    Ok(())
}

pub fn set_step(conn: &mut Connection, id: &str, step: ExplainStep) -> AppResult<bool> {
    let changed = conn.execute(
        "UPDATE explanations SET step = ?2 WHERE id = ?1 AND state = 'working'
           AND (step IS NULL OR step <> ?2)",
        params![id, word(&step)?],
    )?;
    Ok(changed > 0)
}

/// How an explanation ended.
pub struct Ending {
    pub state: ExplanationState,
    pub error: Option<String>,
    pub errors: Vec<String>,
    pub explanation: Option<Explanation>,
    pub cost: Option<ExplainCost>,
    pub duration_secs: Option<u32>,
}

/// End a working explanation; `false` when it had already ended.
pub fn end(conn: &mut Connection, id: &str, ending: &Ending) -> AppResult<bool> {
    let explanation = ending
        .explanation
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let changed = conn.execute(
        "UPDATE explanations SET state = ?2, step = NULL, error = ?3, errors = ?4,
           explanation = ?5, cost_micros = ?6, currency = ?7, duration_secs = ?8, ended_at = ?9
         WHERE id = ?1 AND state = 'working'",
        params![
            id,
            word(&ending.state)?,
            ending.error,
            serde_json::to_string(&ending.errors)?,
            explanation,
            ending.cost.as_ref().map(|c| c.micros as i64),
            ending.cost.as_ref().map(|c| c.currency.clone()),
            ending.duration_secs,
            now_rfc3339(),
        ],
    )?;
    Ok(changed > 0)
}

/// A cost or duration that came after the ending was written.
pub fn set_usage(
    conn: &mut Connection,
    id: &str,
    cost: Option<&ExplainCost>,
    duration_secs: Option<u32>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE explanations SET cost_micros = COALESCE(?2, cost_micros),
           currency = COALESCE(?3, currency), duration_secs = COALESCE(?4, duration_secs)
         WHERE id = ?1",
        params![
            id,
            cost.map(|c| c.micros as i64),
            cost.map(|c| c.currency.clone()),
            duration_secs
        ],
    )?;
    Ok(())
}

pub fn set_hidden(conn: &mut Connection, id: &str, hidden: &[u32]) -> AppResult<()> {
    conn.execute(
        "UPDATE explanations SET hidden = ?2 WHERE id = ?1",
        params![id, serde_json::to_string(hidden)?],
    )?;
    Ok(())
}

pub fn delete(conn: &mut Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM explanations WHERE id = ?1", [id])?;
    Ok(())
}

/// The durations and costs of the last finished explanations with this
/// profile, model, and depth, newest first.
pub fn recent_usage(
    conn: &mut Connection,
    profile_id: &str,
    model: &str,
    depth: ExplainDepth,
    limit: u32,
) -> AppResult<Vec<(u32, Option<ExplainCost>)>> {
    let mut stmt = conn.prepare(
        "SELECT duration_secs, cost_micros, currency FROM explanations
         WHERE profile_id = ?1 AND model = ?2 AND depth = ?3 AND state = 'ready'
           AND duration_secs IS NOT NULL
         ORDER BY ended_at DESC LIMIT ?4",
    )?;
    let rows = stmt
        .query_map(params![profile_id, model, depth.as_str(), limit], |r| {
            let micros: Option<i64> = r.get(1)?;
            let currency: Option<String> = r.get(2)?;
            Ok((
                r.get::<_, u32>(0)?,
                micros.map(|m| ExplainCost {
                    micros: m.max(0) as u64,
                    currency: currency.unwrap_or_else(|| "USD".into()),
                }),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// brainiac.db: answers, known concepts, settings
// ---------------------------------------------------------------------------

pub fn load_settings(conn: &Connection) -> AppResult<ExplanationSettings> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            [SETTINGS_KEY],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

pub fn save_settings(conn: &Connection, settings: &ExplanationSettings) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value_json, version) VALUES (?1, ?2, 1)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, version = version + 1",
        params![SETTINGS_KEY, serde_json::to_string(settings)?],
    )?;
    Ok(())
}

pub fn answers(conn: &Connection) -> AppResult<Vec<ExplainAnswer>> {
    let mut stmt = conn.prepare(
        "SELECT scope, scope_id, provider, allowed, answered_at FROM explain_answers
         ORDER BY scope, scope_id, provider",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ExplainAnswer {
                scope: parse(r.get(0)?)?,
                scope_id: r.get(1)?,
                scope_name: None,
                provider: parse(r.get(2)?)?,
                allowed: r.get(3)?,
                answered_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn set_answer(
    conn: &Connection,
    scope: ExplainAnswerScope,
    scope_id: &str,
    provider: AgentProvider,
    allowed: bool,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO explain_answers (scope, scope_id, provider, allowed, answered_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (scope, scope_id, provider)
         DO UPDATE SET allowed = excluded.allowed, answered_at = excluded.answered_at",
        params![
            word(&scope)?,
            scope_id,
            provider.as_str(),
            allowed,
            now_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn remove_answer(
    conn: &Connection,
    scope: ExplainAnswerScope,
    scope_id: &str,
    provider: AgentProvider,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM explain_answers WHERE scope = ?1 AND scope_id = ?2 AND provider = ?3",
        params![word(&scope)?, scope_id, provider.as_str()],
    )?;
    Ok(())
}

/// A concept's identity: its name folded to lowercase, with spaces and
/// punctuation collapsed to single spaces.
pub fn concept_key(name: &str) -> String {
    let mut key = String::new();
    let mut gap = false;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if gap && !key.is_empty() {
                key.push(' ');
            }
            gap = false;
            key.extend(c.to_lowercase());
        } else {
            gap = true;
        }
    }
    key
}

pub fn concepts(conn: &Connection) -> AppResult<Vec<KnownConcept>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, repository_id, merged_into, learned_at FROM known_concepts
         ORDER BY kind, name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let repository: String = r.get(3)?;
            Ok(KnownConcept {
                id: r.get(0)?,
                kind: parse(r.get(1)?)?,
                name: r.get(2)?,
                repository_id: (!repository.is_empty()).then_some(repository),
                repository_name: None,
                merged_into: r.get(4)?,
                learned_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Got it: add a concept, or return the one already known by that identity.
pub fn add_concept(
    conn: &Connection,
    kind: ConceptKind,
    name: &str,
    repository_id: Option<&str>,
) -> AppResult<String> {
    let name = name.trim();
    let key = concept_key(name);
    if key.is_empty() {
        return Err(AppError::validation("A concept needs a name."));
    }
    let repository = match kind {
        ConceptKind::ProjectPattern => repository_id
            .filter(|r| !r.is_empty())
            .ok_or_else(|| AppError::validation("A project pattern needs its repository."))?,
        _ => "",
    };
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM known_concepts WHERE kind = ?1 AND key = ?2 AND repository_id = ?3",
            params![word(&kind)?, key, repository],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO known_concepts (id, kind, name, key, repository_id, learned_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, word(&kind)?, name, key, repository, now_rfc3339()],
    )?;
    Ok(id)
}

/// Undo or Remove: concepts merged into it are removed with it.
pub fn remove_concept(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM known_concepts WHERE merged_into = ?1", [id])?;
    conn.execute("DELETE FROM known_concepts WHERE id = ?1", [id])?;
    Ok(())
}

/// Merge `from` into `into`: one idea that explanations named in two ways.
pub fn merge_concept(conn: &Connection, from: &str, into: &str) -> AppResult<()> {
    if from == into {
        return Err(AppError::validation(
            "Choose another concept to merge into.",
        ));
    }
    let target_merged: Option<Option<String>> = conn
        .query_row(
            "SELECT merged_into FROM known_concepts WHERE id = ?1",
            [into],
            |r| r.get(0),
        )
        .optional()?;
    let into = match target_merged {
        None => return Err(AppError::not_found("There is no such concept.")),
        Some(Some(root)) => root,
        Some(None) => into.to_string(),
    };
    conn.execute(
        "UPDATE known_concepts SET merged_into = ?2 WHERE id = ?1 OR merged_into = ?1",
        params![from, into],
    )?;
    Ok(())
}

/// What a repository's prompt is told the reader knows: languages,
/// libraries, and system tools from anywhere, and this repository's own
/// project patterns, as "name (kind)".
pub fn known_for(conn: &Connection, repository_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name, kind FROM known_concepts
         WHERE repository_id = '' OR repository_id = ?1
         ORDER BY learned_at DESC",
    )?;
    let rows = stmt
        .query_map([repository_id], |r| {
            let kind: String = r.get(1)?;
            Ok(format!(
                "{} ({})",
                r.get::<_, String>(0)?,
                kind.replace('_', " ")
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concept_keys_fold_case_spaces_and_punctuation() {
        assert_eq!(concept_key("  Arc<Mutex<_>> "), "arc mutex");
        assert_eq!(concept_key("Result / ?"), "result");
        assert_eq!(concept_key("ts-rs"), "ts rs");
        assert_eq!(concept_key("!!"), "");
    }
}
