//! A routine's schedule from the Mac's side (docs/design/routines-ux.md,
//! Schedules). The user sets a schedule and approves it; Brainiac then sends
//! the host what it needs (the agent's key, the code, the schedule), and
//! afterwards only listens: it takes the briefings the host left, notices a
//! host that has gone quiet, and turns the schedule off on the host whenever
//! something it was approved for changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use super::{insert_or_ignore, previous_keys, RoutineService};
use crate::agents::controller::protocol::{
    PutSchedule, ScheduleDays, ScheduleSpec, ScheduleStatus, ScheduledResult, ScheduledState,
};
use crate::agents::controller::schedule::parse_time;
use crate::agents::runtime::RunRuntime;
use crate::agents::settings::{credential_key, image_for};
use crate::models::{
    now_rfc3339, AgentHost, AgentProfile, AppError, AppResult, BriefingFinding, BriefingResult,
    BriefingState, ErrorCode, Routine, RoutineSchedule, ScheduleDaysKind, ScheduleState,
    ScheduleWhen, SetScheduleRequest,
};

/// A run is overdue when it was due this long ago and the host has not been
/// heard from since.
const OVERDUE_AFTER_MINUTES: i64 = 30;

/// A routine's schedule columns.
#[derive(Debug, Clone)]
pub(super) struct Sched {
    pub days: Option<String>,
    pub time: Option<String>,
    pub late_start: bool,
    pub paused: bool,
    pub ceiling: i64,
    pub approved_parts: Option<String>,
    pub approved_at: Option<String>,
    pub host_next_fire: Option<String>,
    pub host_synced_at: Option<String>,
    pub code_commit: Option<String>,
    pub code_at: Option<String>,
    pub memory_unsent: bool,
    pub notice: Option<String>,
}

pub(super) const SCHED_COLUMNS: &str = "schedule_days, schedule_time, late_start, \
    paused, cost_ceiling_micros, approved_parts, approved_at, host_next_fire, host_synced_at, \
    code_commit, code_at, memory_unsent, notice";

pub(super) fn sched_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Sched> {
    Ok(Sched {
        days: r.get(0)?,
        time: r.get(1)?,
        late_start: r.get::<_, i64>(2)? != 0,
        paused: r.get::<_, i64>(3)? != 0,
        ceiling: r.get(4)?,
        approved_parts: r.get(5)?,
        approved_at: r.get(6)?,
        host_next_fire: r.get(7)?,
        host_synced_at: r.get(8)?,
        code_commit: r.get(9)?,
        code_at: r.get(10)?,
        memory_unsent: r.get::<_, i64>(11)? != 0,
        notice: r.get(12)?,
    })
}

impl Sched {
    fn scheduled(&self) -> bool {
        self.days.is_some() && self.time.is_some()
    }

    fn when(&self) -> Option<ScheduleWhen> {
        let (days, weekday) = match self.days.as_deref()? {
            "daily" => (ScheduleDaysKind::Daily, 0),
            "weekdays" => (ScheduleDaysKind::Weekdays, 0),
            other => (
                ScheduleDaysKind::Weekly,
                other.strip_prefix("weekly:")?.parse().ok()?,
            ),
        };
        Some(ScheduleWhen {
            days,
            weekday,
            time: self.time.clone()?,
            late_start: self.late_start,
            cost_ceiling_micros: self.ceiling,
        })
    }

    fn controller_days(&self) -> Option<ScheduleDays> {
        let when = self.when()?;
        Some(match when.days {
            ScheduleDaysKind::Daily => ScheduleDays::Daily,
            ScheduleDaysKind::Weekdays => ScheduleDays::Weekdays,
            ScheduleDaysKind::Weekly => ScheduleDays::Weekly(when.weekday),
        })
    }
}

fn days_text(when: &ScheduleWhen) -> String {
    match when.days {
        ScheduleDaysKind::Daily => "daily".into(),
        ScheduleDaysKind::Weekdays => "weekdays".into(),
        ScheduleDaysKind::Weekly => format!("weekly:{}", when.weekday),
    }
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl RoutineService {
    // -----------------------------------------------------------------------
    // What the user approved
    // -----------------------------------------------------------------------

    /// A digest for each thing an unattended run depends on. The prompt,
    /// the code, the host, the agent, what it can use, the limits and the
    /// schedule itself; the name, the memory and earlier briefings are not
    /// among them.
    async fn parts(&self, routine: &Routine, s: &Sched) -> AppResult<BTreeMap<String, String>> {
        let settings = self.agent_settings.get().await?;
        let profile = settings
            .profiles
            .iter()
            .find(|p| p.id == routine.profile_id);
        let secrets = self
            .agent_settings
            .profile_secret_names(&routine.profile_id)
            .await?;
        let mut parts = BTreeMap::new();
        parts.insert(
            "the code to run".to_string(),
            digest(&format!("{}|{}", routine.repository_id, routine.start)),
        );
        parts.insert("the prompt".to_string(), digest(&routine.prompt));
        parts.insert("the host".to_string(), digest(&routine.host_id));
        parts.insert(
            "the agent and who pays".to_string(),
            digest(&match profile {
                Some(p) => format!("{}|{:?}|{:?}", p.id, p.provider, p.payment),
                None => routine.profile_id.clone(),
            }),
        );
        parts.insert(
            "the secrets and tools it gets".to_string(),
            digest(&format!(
                "{}|{}",
                secrets.join(","),
                profile.is_some_and(|p| p.tools)
            )),
        );
        parts.insert(
            "the limits".to_string(),
            digest(&format!("{}|{}", routine.time_limit_minutes, s.ceiling)),
        );
        parts.insert(
            "the schedule".to_string(),
            digest(&format!(
                "{}|{}|{}",
                s.days.clone().unwrap_or_default(),
                s.time.clone().unwrap_or_default(),
                s.late_start
            )),
        );
        Ok(parts)
    }

    /// What changed since the schedule was approved; every part when it never was.
    async fn changed(&self, routine: &Routine, s: &Sched) -> AppResult<Vec<String>> {
        let now = self.parts(routine, s).await?;
        let approved: BTreeMap<String, String> = s
            .approved_parts
            .as_deref()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default();
        Ok(now
            .iter()
            .filter(|(label, value)| approved.get(*label) != Some(*value))
            .map(|(label, _)| label.clone())
            .collect())
    }

    // -----------------------------------------------------------------------
    // The view
    // -----------------------------------------------------------------------

    pub(super) async fn view(
        &self,
        routine: &Routine,
        s: &Sched,
    ) -> AppResult<Option<RoutineSchedule>> {
        let Some(when) = s.when() else {
            return Ok(None);
        };
        let changed = if s.approved_parts.is_none() {
            vec!["a new schedule".to_string()]
        } else {
            self.changed(routine, s).await?
        };
        let error = self
            .sync_errors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&routine.id)
            .cloned();
        let state = if !changed.is_empty() {
            ScheduleState::NeedsApproval
        } else if s.paused {
            ScheduleState::Paused
        } else if overdue(s) {
            ScheduleState::Overdue
        } else if error.is_some() {
            ScheduleState::Unreachable
        } else {
            ScheduleState::Active
        };
        let month = month_prefix();
        let rid = routine.id.clone();
        let month_cost = self
            .history
            .call(move |conn| {
                Ok(conn.query_row(
                    "SELECT COALESCE(SUM(cost_micros), 0) FROM briefing_results
                     WHERE routine_id = ?1 AND created_at LIKE ?2 AND cost_currency = 'USD'",
                    params![rid, format!("{month}%")],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await?;
        Ok(Some(RoutineSchedule {
            when,
            state,
            notice: s.notice.clone(),
            changed,
            next_fire: s.host_next_fire.clone(),
            synced_at: s.host_synced_at.clone(),
            code_commit: s.code_commit.clone(),
            code_at: s.code_at.clone(),
            error,
            month_cost_micros: month_cost,
        }))
    }

    pub(super) async fn sched(&self, id: &str) -> AppResult<Sched> {
        let key = id.to_string();
        self.core
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT {SCHED_COLUMNS} FROM routines WHERE id = ?1"),
                    params![key],
                    sched_from_row,
                )
                .optional()?
                .ok_or_else(|| AppError::not_found("That routine no longer exists."))
            })
            .await
    }

    fn set_error(&self, id: &str, message: Option<String>) {
        let mut errors = self.sync_errors.lock().unwrap_or_else(|p| p.into_inner());
        match message {
            Some(m) => {
                errors.insert(id.to_string(), m);
            }
            None => {
                errors.remove(id);
            }
        }
    }

    // -----------------------------------------------------------------------
    // The user's actions
    // -----------------------------------------------------------------------

    /// Add or replace the schedule. It is off until approved.
    pub async fn set_schedule(self: &Arc<Self>, request: SetScheduleRequest) -> AppResult<()> {
        let routine = self.routine(&request.routine_id).await?;
        if routine.host_id.is_empty() {
            return Err(AppError::validation(
                "A schedule runs on a run host, not on this Mac, which sleeps. Choose a host for the routine first.",
            ));
        }
        let when = request.when;
        if parse_time(&when.time).is_none() {
            return Err(AppError::validation("The time is HH:MM, such as 07:00."));
        }
        if when.days == ScheduleDaysKind::Weekly && when.weekday > 6 {
            return Err(AppError::validation("Choose a day of the week."));
        }
        if !(10_000..=100_000_000).contains(&when.cost_ceiling_micros) {
            return Err(AppError::validation(
                "A run's cost ceiling is between $0.01 and $100.",
            ));
        }
        let (id, days, time, late, ceiling) = (
            routine.id.clone(),
            days_text(&when),
            when.time.clone(),
            when.late_start,
            when.cost_ceiling_micros,
        );
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET schedule_days = ?2, schedule_time = ?3, late_start = ?4,
                       cost_ceiling_micros = ?5, notice = NULL, version = version + 1
                     WHERE id = ?1",
                    params![id, days, time, late, ceiling],
                )?;
                Ok(())
            })
            .await?;
        self.after_change(&routine.id).await;
        Ok(())
    }

    /// Remove the schedule here and on the host.
    pub async fn remove_schedule(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let routine = self.routine(id).await?;
        let s = self.sched(id).await?;
        if !s.scheduled() {
            return Ok(());
        }
        if s.approved_parts.is_some() {
            // The host has it: it must be told, or it would go on firing.
            let runtime = self.runs.runtime_of(&routine.host_id).await.map_err(|e| {
                host_down(&e, "The schedule is still there and still runs on the host")
            })?;
            runtime.delete_schedule(id).await.map_err(|e| {
                host_down(&e, "The schedule is still there and still runs on the host")
            })?;
        }
        let key = id.to_string();
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET schedule_days = NULL, schedule_time = NULL,
                       late_start = 0, paused = 0, approved_parts = NULL, approved_at = NULL,
                       host_next_fire = NULL, host_synced_at = NULL, code_commit = NULL,
                       code_at = NULL, memory_unsent = 0, notice = NULL, version = version + 1
                     WHERE id = ?1",
                    params![key],
                )?;
                Ok(())
            })
            .await?;
        self.set_error(id, None);
        Ok(())
    }

    /// Approve the schedule as it is now: send the host the agent's key and
    /// the code, check that the secrets it names are there, and turn it on.
    pub async fn approve(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let routine = self.routine(id).await?;
        let s = self.sched(id).await?;
        if !s.scheduled() {
            return Err(AppError::validation("This routine has no schedule yet."));
        }
        let runtime = self
            .runs
            .runtime_of(&routine.host_id)
            .await
            .map_err(|e| host_down(&e, "Nothing was sent"))?;
        // A secret the agent's runs get has to be on the host already.
        let names = self
            .agent_settings
            .profile_secret_names(&routine.profile_id)
            .await?;
        if !names.is_empty() {
            let held = runtime
                .list_secrets()
                .await
                .map_err(|e| host_down(&e, "Nothing was sent"))?;
            let lacking: Vec<&String> = names
                .iter()
                .filter(|n| !held.iter().any(|h| &h.name == *n))
                .collect();
            if !lacking.is_empty() {
                return Err(AppError::validation(format!(
                    "The host does not hold {}. Share {} on its page first.",
                    lacking
                        .iter()
                        .map(|n| n.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    if lacking.len() == 1 { "it" } else { "them" }
                )));
            }
        }
        let approved_at = now_rfc3339();
        let enabled = !s.paused;
        let with_time = Sched {
            approved_at: Some(approved_at.clone()),
            ..s.clone()
        };
        self.push(&routine, &with_time, &runtime, true, enabled, None)
            .await?;
        let parts = serde_json::to_string(&self.parts(&routine, &s).await?).map_err(|e| {
            AppError::io("The approval could not be kept.").with_details(e.to_string())
        })?;
        let key = id.to_string();
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET approved_parts = ?2, approved_at = ?3, notice = NULL,
                       version = version + 1 WHERE id = ?1",
                    params![key, parts, approved_at],
                )?;
                Ok(())
            })
            .await?;
        self.set_error(id, None);
        let _ = self.sync_host(&routine.host_id).await;
        Ok(())
    }

    /// Pause or resume on the host. If the host cannot be told, nothing changes
    /// here either: a schedule shown paused must be paused.
    pub async fn set_paused(self: &Arc<Self>, id: &str, paused: bool) -> AppResult<()> {
        let routine = self.routine(id).await?;
        let s = self.sched(id).await?;
        if !s.scheduled() {
            return Err(AppError::validation("This routine has no schedule."));
        }
        if !paused && self.changed(&routine, &s).await.map(|c| !c.is_empty())? {
            return Err(AppError::validation(
                "Approve the schedule before it runs again: something it depends on changed.",
            ));
        }
        if s.approved_parts.is_some() {
            let runtime = self
                .runs
                .runtime_of(&routine.host_id)
                .await
                .map_err(|e| host_down(&e, "It is not changed"))?;
            let next = Sched {
                paused,
                ..s.clone()
            };
            self.push(&routine, &next, &runtime, false, !paused, None)
                .await
                .map_err(|e| host_down(&e, "It is not changed"))?;
        }
        let key = id.to_string();
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET paused = ?2, notice = NULL, version = version + 1
                     WHERE id = ?1",
                    params![key, paused],
                )?;
                Ok(())
            })
            .await
    }

    /// After the routine or its schedule changed: tell the host whether the
    /// schedule may still run. Failing to is shown, not hidden.
    pub(super) async fn after_change(self: &Arc<Self>, id: &str) {
        let Ok(s) = self.sched(id).await else { return };
        if !s.scheduled() || s.approved_parts.is_none() {
            return;
        }
        let Ok(routine) = self.routine(id).await else {
            return;
        };
        let enabled = !s.paused
            && self
                .changed(&routine, &s)
                .await
                .map(|c| c.is_empty())
                .unwrap_or(false);
        let result = async {
            let runtime = self.runs.runtime_of(&routine.host_id).await?;
            self.push(&routine, &s, &runtime, false, enabled, None)
                .await
        }
        .await;
        match result {
            Ok(()) => self.set_error(id, None),
            Err(e) => self.set_error(
                id,
                Some(format!(
                    "{} The host may still run the earlier version until it is reached.",
                    e.message
                )),
            ),
        }
    }

    // -----------------------------------------------------------------------
    // Sending to the host
    // -----------------------------------------------------------------------

    fn spec(
        &self,
        routine: &Routine,
        s: &Sched,
        profile: &AgentProfile,
        host: &AgentHost,
        secrets: Vec<String>,
        enabled: bool,
    ) -> AppResult<ScheduleSpec> {
        let image = image_for(profile, host).ok_or_else(|| {
            AppError::validation(format!("Build the image on {} first.", host.name))
        })?;
        let days = s
            .controller_days()
            .ok_or_else(|| AppError::validation("The schedule is not complete."))?;
        Ok(ScheduleSpec {
            routine_id: routine.id.clone(),
            name: routine.name.clone(),
            days,
            time: s.time.clone().unwrap_or_default(),
            late_start: s.late_start,
            enabled,
            profile_id: profile.id.clone(),
            credential_key: credential_key(profile),
            agent: profile.agent,
            provider: profile.provider,
            model: profile.model.clone(),
            image: image.name,
            cpus: profile.cpus,
            memory_mib: profile.memory_mib,
            workspace_gib: profile.workspace_gib,
            time_limit_secs: u64::from(routine.time_limit_minutes) * 60,
            host_secrets: secrets,
            prompt: routine.prompt.clone(),
            previous_results: routine.previous_results,
        })
    }

    /// Send the schedule. `full`: the agent's key and the code too. `known`:
    /// what the host already said it holds, so unchanged code is not sent again.
    async fn push(
        &self,
        routine: &Routine,
        s: &Sched,
        runtime: &RunRuntime,
        full: bool,
        enabled: bool,
        known: Option<&ScheduleStatus>,
    ) -> AppResult<()> {
        let settings = self.agent_settings.get().await?;
        let profile = settings
            .profiles
            .iter()
            .find(|p| p.id == routine.profile_id)
            .cloned()
            .ok_or_else(|| AppError::validation("The routine's agent no longer exists."))?;
        let host = settings
            .hosts
            .iter()
            .find(|h| h.id == routine.host_id)
            .cloned()
            .ok_or_else(|| AppError::validation("The routine's host no longer exists."))?;
        let secrets = self
            .agent_settings
            .profile_secret_names(&profile.id)
            .await?;
        let spec = self.spec(routine, s, &profile, &host, secrets, enabled)?;
        if full {
            let credential = self.runs.model_credential(&profile).await?;
            runtime
                .put_model_credential(&profile.id, credential.key, credential.value)
                .await?;
        }
        let mut bundle_commit = None;
        if full || enabled {
            let commit = self
                .runs
                .resolve_start(&routine.repository_id, &routine.start)
                .await?;
            let held = known.and_then(|k| k.bundle_commit.as_deref());
            if full || held != Some(commit.as_str()) {
                let key = format!("sched-{}", routine.id);
                let path = self
                    .runs
                    .export_code(&routine.repository_id, &commit, &key)
                    .await?;
                runtime.send_schedule_bundle(&routine.id, &path).await?;
                bundle_commit = Some(commit);
            }
        }
        let sent_code = bundle_commit.clone();
        runtime
            .put_schedule(PutSchedule {
                spec,
                memory: s.memory_unsent.then(|| routine.memory.clone()),
                bundle_commit,
                since: s.approved_at.clone(),
            })
            .await?;
        let (id, now) = (routine.id.clone(), now_rfc3339());
        self.core
            .call(move |conn| {
                if let Some(commit) = sent_code {
                    conn.execute(
                        "UPDATE routines SET code_commit = ?2, code_at = ?3 WHERE id = ?1",
                        params![id, commit, now],
                    )?;
                }
                conn.execute(
                    "UPDATE routines SET memory_unsent = 0 WHERE id = ?1",
                    params![id],
                )?;
                Ok(())
            })
            .await
    }

    // -----------------------------------------------------------------------
    // Listening to the host
    // -----------------------------------------------------------------------

    /// Listen to every host that has a schedule on it.
    pub async fn sync_all(self: &Arc<Self>) {
        let hosts: Vec<String> = match self
            .core
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT host_id FROM routines
                     WHERE schedule_days IS NOT NULL AND host_id <> ''",
                )?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
        {
            Ok(hosts) => hosts,
            Err(e) => {
                tracing::warn!(error = %e, "the schedules could not be listed");
                return;
            }
        };
        for host in hosts {
            if let Err(e) = self.sync_host(&host).await {
                tracing::info!(host = %host, error = %e, "a host's schedules were not heard");
            }
        }
    }

    /// Ask one host what its schedules left, take it, and put right what it
    /// lacks: the agent's key, the code, a schedule it lost.
    pub async fn sync_host(self: &Arc<Self>, host_id: &str) -> AppResult<()> {
        // One conversation with a host at a time.
        let _one = self.syncing.lock().await;
        let ids: Vec<String> = {
            let host = host_id.to_string();
            self.core
                .call(move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM routines WHERE host_id = ?1 AND schedule_days IS NOT NULL",
                    )?;
                    let rows = stmt.query_map(params![host], |r| r.get::<_, String>(0))?;
                    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
                })
                .await?
        };
        let runtime = match self.runs.runtime_of(host_id).await {
            Ok(runtime) => runtime,
            Err(e) => {
                for id in &ids {
                    self.set_error(
                        id,
                        Some(format!("The host cannot be reached: {}", e.message)),
                    );
                }
                return Err(e);
            }
        };
        let (statuses, _host_now) = match runtime.schedules().await {
            Ok(answer) => answer,
            Err(e) => {
                for id in &ids {
                    self.set_error(
                        id,
                        Some(format!("The host cannot be reached: {}", e.message)),
                    );
                }
                return Err(e);
            }
        };
        // A schedule the host has for a routine that is gone must not go on firing.
        for status in &statuses {
            if !ids.contains(&status.spec.routine_id) {
                let _ = runtime.delete_schedule(&status.spec.routine_id).await;
            }
        }
        for id in ids {
            let (Ok(routine), Ok(s)) = (self.routine(&id).await, self.sched(&id).await) else {
                continue;
            };
            let status = statuses.iter().find(|st| st.spec.routine_id == id);
            let outcome = self.sync_one(&routine, &s, &runtime, status).await;
            match outcome {
                Ok(()) => self.set_error(&id, None),
                Err(e) => self.set_error(&id, Some(e.message)),
            }
        }
        Ok(())
    }

    async fn sync_one(
        self: &Arc<Self>,
        routine: &Routine,
        s: &Sched,
        runtime: &RunRuntime,
        status: Option<&ScheduleStatus>,
    ) -> AppResult<()> {
        let approved = s.approved_parts.is_some();
        let enabled = approved
            && !s.paused
            && self
                .changed(routine, s)
                .await
                .map(|c| c.is_empty())
                .unwrap_or(false);
        let Some(status) = status else {
            // The host lost it (a reinstall, a new state folder): send it all again.
            if approved {
                return self.push(routine, s, runtime, true, enabled, None).await;
            }
            return Ok(());
        };
        self.import(routine, runtime, status).await?;
        let (id, next, now, commit, at) = (
            routine.id.clone(),
            status.next_fire.clone(),
            now_rfc3339(),
            status.bundle_commit.clone(),
            status.bundle_at.clone(),
        );
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET host_next_fire = ?2, host_synced_at = ?3,
                       code_commit = COALESCE(?4, code_commit), code_at = COALESCE(?5, code_at)
                     WHERE id = ?1",
                    params![id, next, now, commit, at],
                )?;
                Ok(())
            })
            .await?;
        if !approved {
            return Ok(());
        }
        // The memory: the host's is the truth, unless it was edited here and not sent.
        if !s.memory_unsent && status.memory != routine.memory {
            let (id, memory, now) = (routine.id.clone(), status.memory.clone(), now_rfc3339());
            self.core
                .call(move |conn| {
                    conn.execute(
                        "UPDATE routines SET memory = ?2, memory_updated_at = ?3 WHERE id = ?1",
                        params![id, memory, now],
                    )?;
                    Ok(())
                })
                .await?;
        }
        // The key it lost, the code the repository moved past, a memory edit not sent.
        let full = !status.credential;
        let moved = enabled
            && match self
                .runs
                .resolve_start(&routine.repository_id, &routine.start)
                .await
            {
                Ok(commit) => status.bundle_commit.as_deref() != Some(commit.as_str()),
                Err(_) => false,
            };
        let spec_differs = status.spec.enabled != enabled;
        if full || moved || s.memory_unsent || spec_differs {
            self.push(routine, s, runtime, full, enabled, Some(status))
                .await?;
        }
        Ok(())
    }

    /// Take the briefings the host has not yet given: each becomes a result,
    /// its run joins Runs, and the host is told it was taken.
    async fn import(
        self: &Arc<Self>,
        routine: &Routine,
        runtime: &RunRuntime,
        status: &ScheduleStatus,
    ) -> AppResult<()> {
        let start_commit = status.bundle_commit.clone().unwrap_or_default();
        let mut taken = Vec::new();
        for r in status.results.iter().filter(|r| !r.acked) {
            if let Some(run_id) = &r.run_id {
                // A run that cannot be taken into Runs (its row, say) is
                // still a briefing worth keeping.
                if let Err(e) = self
                    .runs
                    .adopt_routine_run(routine, run_id, &start_commit)
                    .await
                {
                    tracing::warn!(run = %run_id, error = %e, "a scheduled run was not listed");
                }
            }
            let result = self.result_of(routine, r).await;
            self.history
                .call(move |conn| insert_or_ignore(conn, &result))
                .await?;
            taken.push(r.id.clone());
        }
        if !taken.is_empty() {
            runtime.ack_schedule_results(&routine.id, taken).await?;
        }
        Ok(())
    }

    async fn result_of(&self, routine: &Routine, r: &ScheduledResult) -> BriefingResult {
        let mut result = BriefingResult {
            id: r.id.clone(),
            routine_id: routine.id.clone(),
            run_id: r.run_id.clone(),
            state: BriefingState::Failed,
            status: None,
            summary: String::new(),
            findings: Vec::new(),
            proposals: Vec::new(),
            error: r.error.clone(),
            started_at: r.started_at.clone(),
            finished_at: r.finished_at.clone(),
            duration_secs: duration(&r.started_at, r.finished_at.as_deref()),
            cost_micros: None,
            cost_currency: None,
        };
        if r.state == ScheduledState::Ready {
            if let Some(draft) = &r.draft {
                let (id, started) = (routine.id.clone(), r.started_at.clone());
                let before = self
                    .history
                    .call(move |conn| previous_keys(conn, &id, &started))
                    .await
                    .unwrap_or_default();
                result.state = BriefingState::Ready;
                result.status = draft.status;
                result.summary = draft.summary.clone();
                result.findings = draft
                    .findings
                    .iter()
                    .cloned()
                    .map(|mut f: BriefingFinding| {
                        f.seen_before = before.contains(&f.key);
                        f
                    })
                    .collect();
                result.proposals = draft.proposals.clone();
                result.error = None;
            }
        }
        result
    }

    // -----------------------------------------------------------------------
    // Cost
    // -----------------------------------------------------------------------

    /// Note what finished runs cost, as the agent reported, and pause a
    /// schedule whose run cost more than its ceiling.
    pub(super) async fn fill_costs(self: &Arc<Self>) {
        let pending: Vec<(String, String, String)> = match self
            .history
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, routine_id, run_id FROM briefing_results
                     WHERE cost_micros IS NULL AND run_id IS NOT NULL AND state <> 'working'
                     ORDER BY created_at DESC LIMIT 40",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
        {
            Ok(p) => p,
            Err(_) => return,
        };
        for (result_id, routine_id, run_id) in pending {
            let Some((micros, currency)) = self.runs.reported_cost(&run_id).await else {
                continue;
            };
            let micros = micros as i64;
            let (id, cur) = (result_id.clone(), currency.clone());
            let _ = self
                .history
                .call(move |conn| {
                    conn.execute(
                        "UPDATE briefing_results SET cost_micros = ?2, cost_currency = ?3
                         WHERE id = ?1",
                        params![id, micros, cur],
                    )?;
                    Ok(())
                })
                .await;
            // Over its ceiling: pause it, and say why.
            let Ok(s) = self.sched(&routine_id).await else {
                continue;
            };
            if currency == "USD" && s.scheduled() && !s.paused && micros > s.ceiling {
                let note = format!(
                    "Paused: a run cost ${:.2}, over its ${:.2} ceiling.",
                    micros as f64 / 1e6,
                    s.ceiling as f64 / 1e6
                );
                if self.set_paused(&routine_id, true).await.is_ok() {
                    let key = routine_id.clone();
                    let _ = self
                        .core
                        .call(move |conn| {
                            conn.execute(
                                "UPDATE routines SET notice = ?2 WHERE id = ?1",
                                params![key, note],
                            )?;
                            Ok(())
                        })
                        .await;
                }
            }
        }
    }
}

/// The host was due to run and has not been heard from since.
fn overdue(s: &Sched) -> bool {
    let (Some(next), now) = (s.host_next_fire.as_deref(), Utc::now()) else {
        return false;
    };
    let Ok(next) = DateTime::parse_from_rfc3339(next) else {
        return false;
    };
    let heard = s
        .host_synced_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok());
    let late = now - next.with_timezone(&Utc) > chrono::Duration::minutes(OVERDUE_AFTER_MINUTES);
    late && heard.is_none_or(|h| h < next)
}

fn month_prefix() -> String {
    Utc::now().format("%Y-%m").to_string()
}

fn duration(start: &str, end: Option<&str>) -> Option<u32> {
    let a = DateTime::parse_from_rfc3339(start).ok()?;
    let b = DateTime::parse_from_rfc3339(end?).ok()?;
    Some((b - a).num_seconds().max(0) as u32)
}

fn host_down(error: &AppError, consequence: &str) -> AppError {
    AppError::new(
        ErrorCode::DependencyUnavailable,
        format!(
            "The host cannot be reached ({}). {consequence}.",
            error.message
        ),
    )
}
