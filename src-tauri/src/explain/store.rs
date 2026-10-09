//! Storage of explanations (docs/architecture.md, Explaining changes —
//! v0.6, Storage): the `explanations` rows of `history.db`, and in
//! `brainiac.db` each repository's or workspace's answer, the known
//! concepts, and Settings → Explanations (one value in `settings`).

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{
    now_rfc3339, AgentKind, AgentPayment, AgentProvider, AppError, AppResult, ConceptKind,
    ExplainCost, ExplainDepth, ExplainStep, ExplainSubject, Explanation, ExplanationSettings,
    ExplanationState, KnownConcept,
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
// brainiac.db: known concepts, settings
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
        "SELECT id, kind, name, repository_id, merged_into, learned_at,
                description, learned_from, learned_in, explanation_id
         FROM known_concepts
         ORDER BY learned_at DESC, name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let repository: String = r.get(3)?;
            let learned_in: String = r.get(8)?;
            Ok(KnownConcept {
                id: r.get(0)?,
                kind: parse(r.get(1)?)?,
                name: r.get(2)?,
                repository_id: (!repository.is_empty()).then_some(repository),
                repository_name: None,
                merged_into: r.get(4)?,
                learned_at: r.get(5)?,
                description: r.get(6)?,
                learned_from: r.get(7)?,
                learned_in: (!learned_in.is_empty()).then_some(learned_in),
                learned_in_name: None,
                explanation_id: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Where a concept was learned, kept with it for Settings → Explanations.
#[derive(Debug, Default, Clone)]
pub struct ConceptOrigin {
    pub description: String,
    pub learned_from: String,
    pub learned_in: String,
    pub explanation_id: Option<String>,
}

/// I know this: add a concept, or return the one already known by that
/// identity (which keeps where it was first learned).
pub fn add_concept(
    conn: &Connection,
    kind: ConceptKind,
    name: &str,
    repository_id: Option<&str>,
    origin: &ConceptOrigin,
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
        "INSERT INTO known_concepts (id, kind, name, key, repository_id, learned_at,
                description, learned_from, learned_in, explanation_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            word(&kind)?,
            name,
            key,
            repository,
            now_rfc3339(),
            origin.description,
            origin.learned_from,
            origin.learned_in,
            origin.explanation_id,
        ],
    )?;
    Ok(id)
}

/// Undo or Remove: concepts merged into it are removed with it.
pub fn remove_concept(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM known_concepts WHERE merged_into = ?1", [id])?;
    conn.execute("DELETE FROM known_concepts WHERE id = ?1", [id])?;
    Ok(())
}

/// The longest name a concept can be given by hand.
const MAX_CONCEPT_NAME: usize = 200;

/// Change a concept's name and kind. What it was called stays as a name that
/// stands for it (a row merged into it, as Merge makes), so a later
/// explanation that names it the old way is still left out and the old name
/// is still listed with it. A concept known in every repository stays so; a
/// project pattern may become one (it stops belonging to its repository, so
/// every repository's explanations leave it out), but a concept known
/// everywhere cannot become a project pattern, which would let explanations
/// of other repositories teach it again.
pub fn edit_concept(conn: &Connection, id: &str, name: &str, kind: ConceptKind) -> AppResult<()> {
    let name = name.trim();
    let key = concept_key(name);
    if key.is_empty() {
        return Err(AppError::validation("A concept needs a name."));
    }
    if name.chars().count() > MAX_CONCEPT_NAME {
        return Err(AppError::validation(format!(
            "A concept's name is at most {MAX_CONCEPT_NAME} characters."
        )));
    }
    let found = conn
        .query_row(
            "SELECT kind, name, key, repository_id, merged_into FROM known_concepts WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    parse::<ConceptKind>(r.get(0)?)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((old_kind, old_name, old_key, repository, merged_into)) = found else {
        return Err(AppError::not_found("There is no such concept."));
    };
    if merged_into.is_some() {
        return Err(AppError::validation(
            "This name stands for another concept: edit that one.",
        ));
    }
    let new_repository = match (old_kind, kind) {
        (_, ConceptKind::ProjectPattern) if old_kind != ConceptKind::ProjectPattern => {
            return Err(AppError::validation(
                "A concept known in every repository cannot become a project pattern: other repositories would explain it again.",
            ));
        }
        (ConceptKind::ProjectPattern, new) if new != ConceptKind::ProjectPattern => "",
        _ => repository.as_str(),
    };
    if kind == old_kind && key == old_key {
        // The same identity: only how it is written changes.
        conn.execute(
            "UPDATE known_concepts SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        return Ok(());
    }
    // The identity may be held by another concept, or by one of this
    // concept's own earlier names: going back to an earlier name takes it
    // back, so that name stops being a separate row.
    let holder = conn
        .query_row(
            "SELECT id, merged_into FROM known_concepts
             WHERE kind = ?1 AND key = ?2 AND repository_id = ?3 AND id <> ?4",
            params![word(&kind)?, key, new_repository, id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let own_earlier_name = match &holder {
        Some((holder_id, Some(into))) if into == id => Some(holder_id.clone()),
        _ => None,
    };
    if holder.is_some() && own_earlier_name.is_none() {
        return Err(AppError::new(
            crate::models::ErrorCode::Conflict,
            "Another concept of that kind already has this name: choose it with this one and use Merge into One.",
        ));
    }
    // One change or none: the concept moves to its new identity and its old
    // one is kept as a name that stands for it.
    let tx = conn.unchecked_transaction()?;
    if let Some(earlier) = &own_earlier_name {
        tx.execute("DELETE FROM known_concepts WHERE id = ?1", [earlier])?;
    }
    tx.execute(
        "UPDATE known_concepts SET kind = ?2, name = ?3, key = ?4, repository_id = ?5 WHERE id = ?1",
        params![id, word(&kind)?, name, key, new_repository],
    )?;
    tx.execute(
        "INSERT INTO known_concepts (id, kind, name, key, repository_id, merged_into, learned_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            uuid::Uuid::new_v4().to_string(),
            word(&old_kind)?,
            old_name,
            old_key,
            repository,
            id,
            now_rfc3339(),
        ],
    )?;
    tx.commit()?;
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

/// A known concept as the checker matches it: its identity key, and the
/// concept a merge points at (the one Undo forgets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownRef {
    pub id: String,
    pub name: String,
    pub kind: ConceptKind,
    pub key: String,
    /// The explanation's words for it. Empty when another repository taught
    /// them: those words can quote that repository, and they stay out of
    /// this run.
    pub description: String,
}

/// What a repository's explanation can leave out because the reader knows
/// it: languages, libraries, protocols, and tools from anywhere and this
/// repository's own project patterns, newest first, as the known-concepts
/// file lists them. A concept merged into another resolves to that one.
pub fn known_refs(conn: &Connection, repository_id: &str) -> AppResult<Vec<KnownRef>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.kind, c.key, c.merged_into, t.name, t.kind,
                c.description, t.description, c.learned_in, t.learned_in
         FROM known_concepts c
         LEFT JOIN known_concepts t ON t.id = c.merged_into
         WHERE c.repository_id = '' OR c.repository_id = ?1
         ORDER BY c.learned_at DESC",
    )?;
    // `?` after `query_map` hands a database error to the caller as an
    // `AppError`; the closure itself returns rusqlite's own error type.
    let rows = stmt
        .query_map([repository_id], |r| {
            let key: String = r.get(3)?;
            // A merged concept stands for the one it was merged into: its id,
            // name, kind, and words, under its own key.
            let (id, name, kind, description, learned_in) =
                if r.get::<_, Option<String>>(4)?.is_some() {
                    (
                        r.get::<_, String>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<String>>(10)?,
                    )
                } else {
                    (
                        r.get(0)?,
                        Some(r.get(1)?),
                        Some(r.get(2)?),
                        Some(r.get(7)?),
                        Some(r.get(9)?),
                    )
                };
            Ok((id, name, kind, key, description, learned_in))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut refs = Vec::new();
    for (id, name, kind, key, description, learned_in) in rows {
        // A merge target that is gone leaves no name: skip the dangling row.
        let (Some(name), Some(kind)) = (name, kind) else {
            continue;
        };
        let mut description = description.unwrap_or_default();
        let learned_in = learned_in.unwrap_or_default();
        // Words from another repository can quote that repository's code.
        // Code sharing is one answer per repository, so they stay out.
        if !learned_in.is_empty() && learned_in != repository_id {
            description.clear();
        }
        refs.push(KnownRef {
            id,
            name,
            kind: parse(kind)?,
            key,
            description,
        });
    }
    Ok(refs)
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
    fn ledger() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../migrations/0010_explanations.sql"))
            .unwrap();
        // The kinds of 0.6.1 (protocol, tool, technique).
        conn.execute_batch(include_str!("../../migrations/0012_concept_kinds.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn known_refs_are_this_repositorys_and_resolve_a_merge_to_its_target() {
        let conn = ledger();
        let add = |kind, name: &str, repo: Option<&str>, learned_in: &str, description: &str| {
            add_concept(
                &conn,
                kind,
                name,
                repo,
                &ConceptOrigin {
                    description: description.to_string(),
                    learned_in: learned_in.to_string(),
                    ..ConceptOrigin::default()
                },
            )
            .unwrap()
        };
        let embed = add(
            ConceptKind::Language,
            "go:embed",
            None,
            "repo-b",
            "Puts a file into the binary.",
        );
        let alias = add(
            ConceptKind::Language,
            "Go embed directive",
            None,
            "repo-a",
            "alias words",
        );
        merge_concept(&conn, &alias, &embed).unwrap();
        add(
            ConceptKind::Language,
            "let",
            None,
            "repo-a",
            "Binds a name.",
        );
        add(
            ConceptKind::ProjectPattern,
            "ledger rule",
            Some("repo-a"),
            "repo-a",
            "This repository's rule.",
        );
        add(
            ConceptKind::ProjectPattern,
            "other client rule",
            Some("repo-b"),
            "repo-b",
            "The other client's rule.",
        );

        let refs = known_refs(&conn, "repo-a").unwrap();
        let keys: Vec<&str> = refs.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains(&"go embed directive"));
        assert!(keys.contains(&"ledger rule"));
        // One repository's pattern never reaches another's explanation.
        assert!(!keys.contains(&"other client rule"));
        // The alias resolves to the concept it was merged into, which Undo forgets.
        let by_alias = refs.iter().find(|r| r.key == "go embed directive").unwrap();
        assert_eq!(
            (by_alias.id.as_str(), by_alias.name.as_str()),
            (embed.as_str(), "go:embed")
        );
        // The alias carries the target's words, and those were learned in
        // another repository, so this run does not get them.
        assert_eq!(by_alias.description, "");
        assert_eq!(
            refs.iter().find(|r| r.key == "let").unwrap().description,
            "Binds a name."
        );
        let there = known_refs(&conn, "repo-b").unwrap();
        assert_eq!(
            there
                .iter()
                .find(|r| r.key == "go embed")
                .unwrap()
                .description,
            "Puts a file into the binary."
        );
    }
    /// A ledger of one language concept, one project pattern, and the reads
    /// the checker and the file make of it.
    fn names_in(conn: &Connection, repository: &str) -> Vec<(String, String, ConceptKind)> {
        let mut found: Vec<_> = known_refs(conn, repository)
            .unwrap()
            .into_iter()
            .map(|r| (r.key, r.name, r.kind))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    fn add(conn: &Connection, kind: ConceptKind, name: &str, repo: Option<&str>) -> String {
        add_concept(conn, kind, name, repo, &ConceptOrigin::default()).unwrap()
    }

    #[test]
    fn a_renamed_concept_keeps_its_old_name_standing_for_it() {
        let conn = ledger();
        let id = add(&conn, ConceptKind::Language, "go embed", None);
        edit_concept(&conn, &id, "embedding files", ConceptKind::Language).unwrap();
        // The new name is the concept; the old one still resolves to it, so
        // an explanation that names it the old way is still left out.
        assert_eq!(
            names_in(&conn, "r"),
            [
                (
                    "embedding files".to_string(),
                    "embedding files".to_string(),
                    ConceptKind::Language
                ),
                (
                    "go embed".to_string(),
                    "embedding files".to_string(),
                    ConceptKind::Language
                ),
            ]
        );
    }

    #[test]
    fn a_new_spelling_of_the_same_name_is_not_a_second_concept() {
        let conn = ledger();
        let id = add(&conn, ConceptKind::Language, "go embed", None);
        edit_concept(&conn, &id, "Go:Embed", ConceptKind::Language).unwrap();
        assert_eq!(known_refs(&conn, "r").unwrap().len(), 1);
        assert_eq!(known_refs(&conn, "r").unwrap()[0].name, "Go:Embed");
    }

    #[test]
    fn a_new_name_and_kind_leave_the_old_ones_as_a_name_for_it() {
        let conn = ledger();
        let id = add(&conn, ConceptKind::Tool, "DKIM", None);
        edit_concept(&conn, &id, "DomainKeys signatures", ConceptKind::Protocol).unwrap();
        let refs = known_refs(&conn, "r").unwrap();
        let keys: Vec<(&str, &str, ConceptKind)> = {
            let mut v: Vec<_> = refs
                .iter()
                .map(|r| (r.key.as_str(), r.name.as_str(), r.kind))
                .collect();
            v.sort_by_key(|k| k.0);
            v
        };
        // Both keys lead to the concept as it is now.
        assert_eq!(
            keys,
            [
                ("dkim", "DomainKeys signatures", ConceptKind::Protocol),
                (
                    "domainkeys signatures",
                    "DomainKeys signatures",
                    ConceptKind::Protocol
                ),
            ]
        );
        assert!(refs.iter().all(|r| r.id == id));
        // Learning the old identity again finds the name that stands for the
        // concept, not a second concept: it is already known.
        let again = add(&conn, ConceptKind::Tool, "DKIM", None);
        assert_ne!(again, id);
        assert_eq!(known_refs(&conn, "r").unwrap().len(), 2);
        // Forgetting the concept forgets the old name with it.
        remove_concept(&conn, &id).unwrap();
        assert!(known_refs(&conn, "r").unwrap().is_empty());
    }

    #[test]
    fn a_project_pattern_can_become_known_everywhere_but_not_the_reverse() {
        let conn = ledger();
        let pattern = add(
            &conn,
            ConceptKind::ProjectPattern,
            "retry with backoff",
            Some("a"),
        );
        // Only repository a is told it before, and no other.
        assert_eq!(names_in(&conn, "a").len(), 1);
        assert!(names_in(&conn, "b").is_empty());
        edit_concept(
            &conn,
            &pattern,
            "retry with backoff",
            ConceptKind::Technique,
        )
        .unwrap();
        // Now every repository's explanations leave it out; the old,
        // repository-only name stands for it in repository a alone.
        assert_eq!(names_in(&conn, "b").len(), 1);
        assert_eq!(names_in(&conn, "b")[0].2, ConceptKind::Technique);
        assert_eq!(names_in(&conn, "a").len(), 2);
        // The reverse is refused, and so is making a name stand for another.
        let lang = add(&conn, ConceptKind::Language, "traits", None);
        assert!(edit_concept(&conn, &lang, "traits", ConceptKind::ProjectPattern).is_err());
        let alias_id: String = conn
            .query_row(
                "SELECT id FROM known_concepts WHERE merged_into IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(edit_concept(&conn, &alias_id, "x", ConceptKind::Technique).is_err());
    }

    #[test]
    fn an_edit_that_meets_another_concept_asks_for_a_merge_instead() {
        let conn = ledger();
        let a = add(&conn, ConceptKind::Library, "serde", None);
        add(&conn, ConceptKind::Library, "serde json", None);
        let error = edit_concept(&conn, &a, "Serde JSON", ConceptKind::Library).unwrap_err();
        assert!(
            error.message.contains("Merge into One"),
            "{}",
            error.message
        );
        // Nothing changed.
        assert_eq!(known_refs(&conn, "r").unwrap().len(), 2);
        assert!(edit_concept(&conn, &a, "  ", ConceptKind::Library).is_err());
        assert!(edit_concept(&conn, "missing", "x", ConceptKind::Library).is_err());
    }
    #[test]
    fn going_back_to_an_earlier_name_takes_it_back() {
        let conn = ledger();
        let id = add(&conn, ConceptKind::Tool, "DKIM", None);
        edit_concept(&conn, &id, "DKIM", ConceptKind::Protocol).unwrap();
        // Changed its mind: DKIM is a tool after all.
        edit_concept(&conn, &id, "DKIM", ConceptKind::Tool).unwrap();
        let refs = known_refs(&conn, "r").unwrap();
        // The concept is a tool again; "DKIM (protocol)" now stands for it.
        assert!(refs
            .iter()
            .all(|r| r.id == id && r.kind == ConceptKind::Tool));
        assert_eq!(refs.len(), 2);
    }
}
