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
use crate::agents::controller::schedule::{parse_time, RESULTS_PER_ANSWER};
use crate::agents::runtime::RunRuntime;
use crate::agents::settings::{credential_key, image_for};
use crate::models::{
    now_rfc3339, AgentHost, AgentProfile, AppError, AppResult, BriefingFinding, BriefingResult,
    BriefingState, ErrorCode, HostClock, HostScheduleOnly, Routine, RoutineSchedule,
    ScheduleDaysKind, ScheduleState, ScheduleWhen, SetScheduleRequest,
};

/// A run is overdue when it was due this long ago and the host has not been
/// heard from since.
const OVERDUE_AFTER_MINUTES: i64 = 30;
/// A host is shown as unreachable once it has not answered for this long:
/// the second of the looks five minutes apart, not the first.
const UNREACHABLE_AFTER: chrono::Duration = chrono::Duration::minutes(4);

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
    /// Approved, and waiting for the host to be prepared.
    pub preparing: bool,
}

pub(super) const SCHED_COLUMNS: &str = "schedule_days, schedule_time, late_start, \
    paused, cost_ceiling_micros, approved_parts, approved_at, host_next_fire, host_synced_at, \
    code_commit, code_at, memory_unsent, notice, preparing";

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
        preparing: r.get::<_, i64>(13)? != 0,
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
            .secret_names(&routine.secret_ids)
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
        // How big a run is and which model it asks for change what it costs and does.
        let host = settings.hosts.iter().find(|h| h.id == routine.host_id);
        parts.insert(
            "the model and resources".to_string(),
            digest(&match profile {
                Some(p) => format!(
                    "{}|{}|{}|{}",
                    p.model, p.cpus, p.memory_mib, p.workspace_gib
                ),
                None => String::new(),
            }),
        );
        parts.insert(
            "the earlier briefings it sees".to_string(),
            digest(&routine.previous_results.to_string()),
        );
        parts.insert(
            "the secrets and tools it gets".to_string(),
            digest(&format!(
                "{}|{}|{}",
                secrets.join(","),
                routine.tools,
                // The layer's image: a rebuilt one is different code in the run.
                match host {
                    Some(h) if routine.tools => h
                        .tools_image
                        .as_ref()
                        .map(|i| i.id.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                }
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
        let state = if s.preparing {
            ScheduleState::Preparing
        } else if !changed.is_empty() {
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
        check_start(&routine)?;
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

    /// The routines of this Mac scheduled on `host_id`.
    async fn scheduled_on(&self, host_id: &str) -> AppResult<Vec<String>> {
        let host = host_id.to_string();
        self.core
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id FROM routines WHERE host_id = ?1 AND schedule_days IS NOT NULL",
                )?;
                let rows = stmt.query_map(params![host], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
    }

    /// What `host_id` fires for no routine of this Mac there, read from the
    /// host now (without changing anything there, so opening a page costs
    /// one request), or as the last sync found when it does not answer.
    pub async fn host_schedules_only(
        self: &Arc<Self>,
        host_id: &str,
    ) -> AppResult<Vec<HostScheduleOnly>> {
        let read = async {
            let runtime = self.runs.runtime_of(host_id).await?;
            let (statuses, _) = runtime.schedules().await?;
            let ids = self.scheduled_on(host_id).await?;
            let mut only = Vec::new();
            for status in statuses
                .iter()
                .filter(|s| !ids.contains(&s.spec.routine_id))
            {
                match self.routine(&status.spec.routine_id).await {
                    Ok(_) => only.push(only_entry(status, true)),
                    Err(e) if e.code == ErrorCode::NotFound => only.push(only_entry(status, false)),
                    Err(_) => {}
                }
            }
            AppResult::Ok(only)
        };
        match read.await {
            Ok(only) => {
                self.only_on_host
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(host_id.to_string(), only.clone());
                Ok(only)
            }
            Err(_) => Ok(self.only_on_host(host_id)),
        }
    }

    fn only_on_host(&self, host_id: &str) -> Vec<HostScheduleOnly> {
        self.only_on_host
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(host_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The user removes a schedule the host fires for no routine of this Mac
    /// there: the host forgets it, its copy of the code, and the briefings
    /// waiting there. One of this Mac's routines that moved has what it left
    /// taken first, when the host gives it. A routine scheduled on this host
    /// is removed from its own page.
    pub async fn remove_host_schedule_only(
        self: &Arc<Self>,
        host_id: &str,
        routine_id: &str,
    ) -> AppResult<Vec<HostScheduleOnly>> {
        let mine = match self.routine(routine_id).await {
            Ok(routine) => {
                if routine.host_id == host_id && self.sched(routine_id).await?.scheduled() {
                    return Err(AppError::validation(
                        "That routine is scheduled on this host: remove its schedule on its own page.",
                    ));
                }
                Some(routine)
            }
            Err(e) if e.code == ErrorCode::NotFound => None,
            Err(e) => return Err(e),
        };
        let _one = self.syncing.lock().await;
        let runtime =
            self.runs.runtime_of(host_id).await.map_err(|e| {
                host_down(&e, "The schedule is still there and still runs on the host")
            })?;
        if let Some(routine) = &mine {
            if let Ok((statuses, _)) = runtime.schedules().await {
                if let Some(status) = statuses.iter().find(|s| s.spec.routine_id == routine_id) {
                    let _ = self.import(routine, host_id, &runtime, status).await;
                }
            }
        }
        runtime
            .delete_schedule(routine_id)
            .await
            .map_err(|e| host_down(&e, "The schedule is still there and still runs on the host"))?;
        let mut all = self.only_on_host.lock().unwrap_or_else(|p| p.into_inner());
        let list = all.entry(host_id.to_string()).or_default();
        list.retain(|s| s.routine_id != routine_id);
        Ok(list.clone())
    }

    /// Remove the schedule here and on the host.
    pub async fn remove_schedule(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let routine = self.routine(id).await?;
        let s = self.sched(id).await?;
        if !s.scheduled() {
            return Ok(());
        }
        let _one = self.syncing.lock().await;
        // A host that was removed has nothing left to tell.
        let host_exists = self
            .agent_settings
            .get()
            .await?
            .hosts
            .iter()
            .any(|h| h.id == routine.host_id);
        if s.approved_parts.is_some() && host_exists {
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
                       code_at = NULL, memory_unsent = 0, notice = NULL, preparing = 0,
                       version = version + 1
                     WHERE id = ?1",
                    params![key],
                )?;
                Ok(())
            })
            .await?;
        self.set_error(id, None);
        Ok(())
    }

    /// Approve the schedule as it is now. The routine's access goes to the
    /// host first. A host whose helper is older or whose image is not built
    /// is prepared (a host job), and the schedule is sent when that is done;
    /// otherwise the agent's key, the code and the schedule are sent now.
    pub async fn approve(self: &Arc<Self>, id: &str) -> AppResult<()> {
        let routine = self.routine(id).await?;
        let s = self.sched(id).await?;
        if !s.scheduled() {
            return Err(AppError::validation("This routine has no schedule yet."));
        }
        check_start(&routine)?;
        // One conversation with the host at a time, so a sync cannot undo this.
        let one = self.syncing.lock().await;
        let runtime = self
            .runs
            .runtime_of(&routine.host_id)
            .await
            .map_err(|e| host_down(&e, "Nothing was sent"))?;
        if !routine.secret_ids.is_empty() {
            self.named
                .ensure_on_host(&routine.secret_ids, &routine.host_id, &runtime)
                .await?;
        }
        let host = self.host_of(&routine).await?;
        if needs_preparing(&host, &routine) {
            self.set_preparing(id, true, None).await?;
            drop(one);
            match self
                .jobs
                .start(
                    &routine.host_id,
                    crate::models::HostJobKind::Prepare,
                    Some(routine.profile_id.clone()),
                )
                .await
            {
                Ok(_) => {}
                // Another job on the host: wait for it, then look again.
                Err(e) if e.code == ErrorCode::Conflict => {}
                Err(e) => {
                    self.set_preparing(id, false, None).await?;
                    return Err(e);
                }
            }
            self.wait_prepared(&routine.host_id);
            return Ok(());
        }
        let done = self.send_approved(&routine, &s, &runtime).await;
        drop(one);
        done?;
        let _ = self.sync_host(&routine.host_id).await;
        Ok(())
    }

    /// Send what was approved and record the approval: the agent's key, the
    /// code and the schedule, turned on unless it is paused.
    async fn send_approved(
        &self,
        routine: &Routine,
        s: &Sched,
        runtime: &RunRuntime,
    ) -> AppResult<()> {
        let approved_at = now_rfc3339();
        let with_time = Sched {
            approved_at: Some(approved_at.clone()),
            ..s.clone()
        };
        self.push(routine, &with_time, runtime, true, !s.paused, None)
            .await?;
        // Taken after the host was prepared, so a newly built image is part
        // of what was approved.
        let parts = serde_json::to_string(&self.parts(routine, s).await?).map_err(|e| {
            AppError::io("The approval could not be kept.").with_details(e.to_string())
        })?;
        let key = routine.id.clone();
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET approved_parts = ?2, approved_at = ?3, notice = NULL,
                       preparing = 0, version = version + 1 WHERE id = ?1",
                    params![key, parts, approved_at],
                )?;
                Ok(())
            })
            .await?;
        self.set_error(&routine.id, None);
        Ok(())
    }

    async fn host_of(&self, routine: &Routine) -> AppResult<AgentHost> {
        self.agent_settings
            .get()
            .await?
            .hosts
            .into_iter()
            .find(|h| h.id == routine.host_id)
            .ok_or_else(|| AppError::validation("The routine's host no longer exists."))
    }

    async fn set_preparing(&self, id: &str, on: bool, notice: Option<String>) -> AppResult<()> {
        let key = id.to_string();
        self.core
            .call(move |conn| {
                conn.execute(
                    "UPDATE routines SET preparing = ?2, notice = ?3, version = version + 1
                     WHERE id = ?1",
                    params![key, on, notice],
                )?;
                Ok(())
            })
            .await
    }

    /// Wait in the background for the host's job to end, then finish the
    /// approvals that waited for it.
    fn wait_prepared(self: &Arc<Self>, host_id: &str) {
        let service = Arc::clone(self);
        let host = host_id.to_string();
        tokio::spawn(async move {
            while service.jobs.busy(&host) {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            service.finish_preparing(&host).await;
        });
    }

    /// Every routine on this host waiting to be prepared: send it now if the
    /// host is ready, or say why not and leave it unapproved.
    pub(super) async fn finish_preparing(self: &Arc<Self>, host_id: &str) {
        if self.jobs.busy(host_id) {
            return;
        }
        let host = host_id.to_string();
        let Ok(ids) = self
            .core
            .call(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT id FROM routines WHERE host_id = ?1 AND preparing = 1")?;
                let rows = stmt.query_map(params![host], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
        else {
            return;
        };
        for id in ids {
            let one = self.syncing.lock().await;
            // `Ok(true)`: a preparation for this routine started; the rest
            // wait for it.
            let result: AppResult<bool> = async {
                let routine = self.routine(&id).await?;
                let s = self.sched(&id).await?;
                if !s.preparing {
                    return Ok(false);
                }
                let host = self.host_of(&routine).await?;
                if needs_preparing(&host, &routine) {
                    let last = self
                        .jobs
                        .list()
                        .into_iter()
                        .find(|j| j.host_id == routine.host_id);
                    let ours = last.as_ref().is_some_and(|j| {
                        j.kind == crate::models::HostJobKind::Prepare
                            && j.profile_ids.contains(&routine.profile_id)
                    });
                    if !ours {
                        // What ran was another's (a test, a build, another
                        // routine's agent): prepare for this one now.
                        return match self
                            .jobs
                            .start(
                                &routine.host_id,
                                crate::models::HostJobKind::Prepare,
                                Some(routine.profile_id.clone()),
                            )
                            .await
                        {
                            Ok(_) => Ok(true),
                            Err(e) if e.code == ErrorCode::Conflict => Ok(true),
                            Err(e) => Err(e),
                        };
                    }
                    let why = last
                        .and_then(|j| j.error)
                        .unwrap_or_else(|| "it is not ready yet.".into());
                    let stop = if why.ends_with('.') { "" } else { "." };
                    return Err(AppError::dependency(format!(
                        "{} could not be prepared: {why}{stop} Approve again to retry.",
                        host.name
                    )));
                }
                let runtime = self.runs.runtime_of(&routine.host_id).await?;
                self.send_approved(&routine, &s, &runtime).await?;
                Ok(false)
            }
            .await;
            drop(one);
            match result {
                Ok(true) => {
                    self.wait_prepared(host_id);
                    return;
                }
                Ok(false) => {}
                Err(e) => {
                    let _ = self.set_preparing(&id, false, Some(e.message)).await;
                }
            }
        }
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
        let _one = self.syncing.lock().await;
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
        if s.preparing {
            // Nothing is on the host yet; the approval's end sends it.
            return;
        }
        let _one = self.syncing.lock().await;
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
        let image = image_for(routine.tools, host).ok_or_else(|| {
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
            .secret_names(&routine.secret_ids)
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
        // Every installed host too: one a routine moved away from may still
        // fire it, and only a sync of that host stops it.
        let mut hosts = hosts;
        if let Ok(settings) = self.agent_settings.get().await {
            for h in settings.hosts {
                if h.kind == "ssh" && h.installed && h.approved && !hosts.contains(&h.id) {
                    hosts.push(h.id);
                }
            }
        }
        for host in hosts {
            // An approval that waited for the host, left by a quit or a restart.
            self.finish_preparing(&host).await;
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
        let ids = self.scheduled_on(host_id).await?;
        let runtime = match self.runs.runtime_of(host_id).await {
            Ok(runtime) => runtime,
            Err(e) => {
                self.not_answered(host_id, &ids, &e);
                return Err(e);
            }
        };
        let (statuses, host_now) = match runtime.schedules().await {
            Ok(answer) => answer,
            Err(e) => {
                self.not_answered(host_id, &ids, &e);
                return Err(e);
            }
        };
        self.note_clock(host_id, &host_now);
        self.unanswered_since
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(host_id);
        // What the host fires for no routine of this host here. A routine
        // this Mac has that moved (or whose schedule here went) is ours: what
        // it left comes home, then it stops here; if either cannot be done,
        // it is listed so the user can stop it. One this Mac has never had
        // (another Mac's, or one from after the backup this Mac was restored
        // from) stays, results and all, and is listed for the user to remove.
        let mut only = Vec::new();
        for status in statuses
            .iter()
            .filter(|s| !ids.contains(&s.spec.routine_id))
        {
            match self.routine(&status.spec.routine_id).await {
                Ok(moved) => {
                    let stopped = self.import(&moved, host_id, &runtime, status).await.is_ok()
                        && runtime
                            .delete_schedule(&status.spec.routine_id)
                            .await
                            .is_ok();
                    if !stopped {
                        only.push(only_entry(status, true));
                    }
                }
                Err(e) if e.code == ErrorCode::NotFound => only.push(only_entry(status, false)),
                Err(_) => {}
            }
        }
        self.only_on_host
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(host_id.to_string(), only);
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

    /// A host that did not answer. Its routines say it cannot be reached only
    /// once it has been silent for two looks in a row, so one dropped
    /// connection does not mark every routine there.
    fn not_answered(&self, host_id: &str, ids: &[String], error: &AppError) {
        let now = Utc::now();
        let first = *self
            .unanswered_since
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(host_id.to_string())
            .or_insert(now);
        if now - first < UNREACHABLE_AFTER {
            return;
        }
        for id in ids {
            self.set_error(
                id,
                Some(format!("The host cannot be reached: {}", error.message)),
            );
        }
    }

    /// A host's clock, which a schedule's time is read in: asked of the host,
    /// or as it last answered when it cannot be reached now.
    pub async fn host_clock(&self, host_id: &str) -> AppResult<HostClock> {
        let asked = async {
            let runtime = self.runs.runtime_of(host_id).await?;
            runtime.schedules().await
        }
        .await;
        match asked {
            Ok((_, now)) => {
                self.note_clock(host_id, &now);
            }
            Err(e) => tracing::info!(host = %host_id, error = %e, "a host's clock was not read"),
        }
        self.clocks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(host_id)
            .cloned()
            .ok_or_else(|| AppError::dependency("The host's time is not known yet."))
    }

    fn note_clock(&self, host_id: &str, now: &str) {
        let Ok(at) = DateTime::parse_from_rfc3339(now) else {
            return;
        };
        self.clocks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                host_id.to_string(),
                HostClock {
                    now: now.to_string(),
                    utc_offset_minutes: at.offset().local_minus_utc() / 60,
                },
            );
    }

    /// Count a host as silent since `at`, as if the looks before had not been
    /// answered. For tests, which cannot wait for them.
    pub fn host_silent_since(&self, host_id: &str, at: DateTime<Utc>) {
        self.unanswered_since
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(host_id.to_string(), at);
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
            // The host lost it (a reinstall, a new state folder): send it all
            // again, the memory too, or the host starts from none and the next
            // sync takes that back.
            if approved {
                let all = Sched {
                    memory_unsent: true,
                    ..s.clone()
                };
                return self.push(routine, &all, runtime, true, enabled, None).await;
            }
            return Ok(());
        };
        self.import(routine, &routine.host_id, runtime, status)
            .await?;
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
    /// `host_id` is the host the status came from: the routine's, unless it
    /// moved since.
    async fn import(
        self: &Arc<Self>,
        routine: &Routine,
        host_id: &str,
        runtime: &RunRuntime,
        status: &ScheduleStatus,
    ) -> AppResult<()> {
        let start_commit = status.bundle_commit.clone().unwrap_or_default();
        let mut batch = status.clone();
        // The host sends a few at a time; keep asking while it has more.
        for _ in 0..8 {
            let more = self
                .import_batch(routine, host_id, runtime, &batch, &start_commit)
                .await?
                >= RESULTS_PER_ANSWER;
            if !more {
                break;
            }
            let (all, _) = runtime.schedules().await?;
            match all.into_iter().find(|s| s.spec.routine_id == routine.id) {
                Some(next) => batch = next,
                None => break,
            }
        }
        Ok(())
    }

    /// Take one batch; how many it held.
    async fn import_batch(
        self: &Arc<Self>,
        routine: &Routine,
        host_id: &str,
        runtime: &RunRuntime,
        status: &ScheduleStatus,
        start_commit: &str,
    ) -> AppResult<usize> {
        let mut taken = Vec::new();
        for r in status.results.iter().filter(|r| !r.acked) {
            if let Some(run_id) = &r.run_id {
                // A run that cannot be taken into Runs (its row, say) is
                // still a briefing worth keeping.
                if let Err(e) = self
                    .runs
                    .adopt_routine_run(routine, host_id, run_id, start_commit)
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
        let count = taken.len();
        if !taken.is_empty() {
            runtime.ack_schedule_results(&routine.id, taken).await?;
        }
        Ok(count)
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
                       AND created_at > datetime('now', '-21 days')
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
                // Nothing reported, and the run is over: it will not be.
                if self
                    .runs
                    .get(&run_id)
                    .await
                    .is_ok_and(|r| r.phase == crate::models::RunPhase::Ended)
                {
                    self.store_cost(&result_id, 0, "none").await;
                }
                continue;
            };
            let micros = micros as i64;
            // Over its ceiling: pause it first, and keep the cost unrecorded
            // when that fails, so the next look tries again.
            if let Ok(s) = self.sched(&routine_id).await {
                if currency == "USD" && s.scheduled() && !s.paused && micros > s.ceiling {
                    let note = format!(
                        "Paused: a run cost ${:.2}, over its ${:.2} ceiling.",
                        micros as f64 / 1e6,
                        s.ceiling as f64 / 1e6
                    );
                    match self.set_paused(&routine_id, true).await {
                        Ok(()) => {
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
                        Err(e) => {
                            self.set_error(
                                &routine_id,
                                Some(format!(
                                    "A run cost more than its ceiling, but the schedule could not be paused: {}",
                                    e.message
                                )),
                            );
                            continue;
                        }
                    }
                }
            }
            self.store_cost(&result_id, micros, &currency).await;
        }
    }

    async fn store_cost(&self, result_id: &str, micros: i64, currency: &str) {
        let (id, cur) = (result_id.to_string(), currency.to_string());
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
    }
}

/// A schedule starts from a named branch or a commit, never from `HEAD`: what
/// the host runs must not depend on what is checked out here.
fn check_start(routine: &Routine) -> AppResult<()> {
    if routine.start.trim().eq_ignore_ascii_case("HEAD") {
        return Err(AppError::validation(
            "A schedule starts from a branch or a commit, not HEAD, so what the host runs does not depend on what is checked out here. Change Start from in Definition.",
        ));
    }
    Ok(())
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

/// A host must be prepared before a schedule can be sent to it: its helper
/// is older than this Brainiac, or the image the routine's runs use is not
/// built as this Brainiac would build it.
/// A host's schedule as the host's page lists it when it is no routine of
/// this Mac there.
fn only_entry(status: &ScheduleStatus, mine: bool) -> HostScheduleOnly {
    HostScheduleOnly {
        routine_id: status.spec.routine_id.clone(),
        name: status.spec.name.clone(),
        enabled: status.spec.enabled,
        next_fire: status.next_fire.clone(),
        results_waiting: status.results.iter().filter(|r| !r.acked).count() as u32,
        mine,
    }
}

fn needs_preparing(host: &AgentHost, routine: &Routine) -> bool {
    let current = |i: &Option<crate::models::AgentImage>| i.as_ref().is_some_and(|i| i.current);
    host.upgrade_available
        || !current(&host.image)
        || (routine.tools && !current(&host.tools_image))
        || !crate::agents::host_jobs::tested_here(host, &routine.profile_id)
}
