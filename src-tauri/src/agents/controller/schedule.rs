//! Schedules kept and fired by a run host's controller (docs/design/
//! routines-ux.md, Schedules; docs/design/scheduled-runs.md, The host owns
//! the schedule). The Mac approves a schedule and sends it with the code and
//! the model credential; from then on the host starts the run at the time,
//! follows it, reads the briefing, and keeps what it left until the Mac asks.
//! Nothing here needs the Mac to be awake, and nothing here writes outside
//! the run's own container.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Datelike, Duration as Span, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

use super::docker::Workloads;
use super::protocol::{
    valid_id, Credential, Phase, PutSchedule, ScheduleDays, ScheduleSpec, ScheduleStatus,
    ScheduledResult, ScheduledState, StartRun, StopReason, MAX_EXTRA_SECRETS,
};
use super::state::write_atomic;
use super::{check_host_secret_names, conflict, now, Activity, Controller};
use crate::agents::routines::{self, Draft};
use crate::models::{AppError, AppResult, BriefingResult, BriefingState, RunPermissions};

/// A slot is started on time if the host sees it this soon after the time.
/// Later than this it was missed (or started late, when the schedule says so).
const GRACE: Span = Span::seconds(180);
/// How often the controller looks at its schedules.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// Results kept per schedule; unacknowledged ones are never dropped.
const KEEP_RESULTS: usize = 14;
/// Missed slots recorded after a long outage.
const MAX_MISSED: usize = 7;
/// Most slots looked at in one go.
const MAX_SLOTS: usize = 400;

/// What the controller keeps about one schedule, on disk, one file each.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ScheduleRecord {
    pub spec: ScheduleSpec,
    pub memory: String,
    pub memory_updated_at: Option<String>,
    pub bundle_commit: Option<String>,
    pub bundle_at: Option<String>,
    /// Slots up to here are decided: fired, missed, or skipped.
    pub cursor: String,
    /// Newest first.
    pub results: Vec<ScheduledResult>,
}

/// A scheduled run being followed: the turn last read, and whether the
/// follow-up turn was sent.
#[derive(Default)]
struct Drive {
    checked_turn: u32,
    followed_up: bool,
}

#[derive(Default)]
pub(super) struct Schedules {
    pub records: HashMap<String, ScheduleRecord>,
    driving: HashMap<String, Drive>,
    /// The slot each run was started for.
    slots: HashMap<String, String>,
    last_look: Option<Instant>,
    last_saved: Option<Instant>,
}

impl Schedules {
    pub fn load(dir: &std::path::Path) -> Self {
        let mut records = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                match std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<ScheduleRecord>(&b).ok())
                {
                    Some(record) => {
                        records.insert(record.spec.routine_id.clone(), record);
                    }
                    None => {
                        tracing::warn!(file = %path.display(), "a schedule's record cannot be read")
                    }
                }
            }
        }
        Schedules {
            records,
            ..Schedules::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

fn day_matches(days: ScheduleDays, weekday: chrono::Weekday) -> bool {
    let index = weekday.num_days_from_monday() as u8;
    match days {
        ScheduleDays::Daily => true,
        ScheduleDays::Weekdays => index < 5,
        ScheduleDays::Weekly(day) => index == day,
    }
}

/// The times in `(after, until]` the schedule is due, oldest first, in local
/// time. A time that does not exist (the clock went forward) is skipped, and
/// one that happens twice (it went back) counts once, the earlier.
pub fn slots_in(
    days: ScheduleDays,
    time: NaiveTime,
    after: DateTime<Local>,
    until: DateTime<Local>,
) -> Vec<DateTime<Local>> {
    let mut out = Vec::new();
    let mut date = after.date_naive();
    let last = until.date_naive();
    while date <= last && out.len() < MAX_SLOTS {
        if day_matches(days, date.weekday()) {
            if let Some(slot) = Local.from_local_datetime(&date.and_time(time)).earliest() {
                if slot > after && slot <= until {
                    out.push(slot);
                }
            }
        }
        match date.succ_opt() {
            Some(next) => date = next,
            None => break,
        }
    }
    out
}

/// The next time after `after` the schedule is due.
pub fn next_after(
    days: ScheduleDays,
    time: NaiveTime,
    after: DateTime<Local>,
) -> Option<DateTime<Local>> {
    slots_in(days, time, after, after + Span::days(8))
        .into_iter()
        .next()
}

fn parse_stamp(text: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Local))
}

// ---------------------------------------------------------------------------
// The controller's side
// ---------------------------------------------------------------------------

fn bundle_key(routine_id: &str) -> String {
    format!("sched-{routine_id}")
}

impl<W: Workloads> Controller<W> {
    fn schedule_dir(&self) -> PathBuf {
        self.state.root().join("schedules")
    }

    fn require_service(&self) -> AppResult<()> {
        if self.service {
            Ok(())
        } else {
            Err(AppError::validation(
                "Only a host's run controller keeps schedules.",
            ))
        }
    }

    fn save_record(&self, record: &ScheduleRecord) {
        let dir = self.schedule_dir();
        let result = std::fs::create_dir_all(&dir)
            .and_then(|()| serde_json::to_vec_pretty(record).map_err(std::io::Error::other))
            .and_then(|bytes| {
                write_atomic(
                    &dir.join(format!("{}.json", record.spec.routine_id)),
                    &bytes,
                )
            });
        if let Err(e) = result {
            tracing::warn!(error = %e, "a schedule's record was not saved");
        }
    }

    fn schedules(&self) -> std::sync::MutexGuard<'_, Schedules> {
        self.schedules
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Keep or replace a schedule.
    pub(super) fn put_schedule(&self, put: PutSchedule) -> AppResult<()> {
        self.require_service()?;
        let PutSchedule {
            spec,
            memory,
            bundle_commit,
            since,
        } = put;
        if !valid_id(&spec.routine_id) {
            return Err(AppError::validation("That routine ID is not valid."));
        }
        if parse_time(&spec.time).is_none() {
            return Err(AppError::validation("A schedule's time is HH:MM."));
        }
        if let ScheduleDays::Weekly(day) = spec.days {
            if day > 6 {
                return Err(AppError::validation("A weekday is 0 to 6."));
            }
        }
        if spec.prompt.trim().is_empty() || spec.prompt.len() > 32 * 1024 {
            return Err(AppError::validation("A schedule needs its prompt."));
        }
        if !(60..=86_400).contains(&spec.time_limit_secs)
            || spec.cpus == 0
            || spec.memory_mib == 0
            || spec.workspace_gib == 0
            || spec.image.is_empty()
            || spec.host_secrets.len() > MAX_EXTRA_SECRETS
        {
            return Err(AppError::validation("A schedule's limits are not valid."));
        }
        check_host_secret_names(&[], &spec.host_secrets)?;
        let since = match since.as_deref() {
            Some(text) => parse_stamp(text)
                .ok_or_else(|| AppError::validation("A schedule's start time is not valid."))?,
            None => Local::now(),
        };
        let bundle_at = if bundle_commit.is_some() {
            let path = self.state.input_bundle(&bundle_key(&spec.routine_id));
            if !path.is_file() {
                return Err(conflict("The code for this schedule has not been sent."));
            }
            Some(now())
        } else {
            None
        };
        let mut schedules = self.schedules();
        let fresh = !schedules.records.contains_key(&spec.routine_id);
        let record = schedules
            .records
            .entry(spec.routine_id.clone())
            .or_insert_with(|| ScheduleRecord {
                spec: spec.clone(),
                memory: String::new(),
                memory_updated_at: None,
                bundle_commit: None,
                bundle_at: None,
                cursor: since.to_rfc3339(),
                results: Vec::new(),
            });
        // Turning a schedule on counts from now: the slots it was off are not owed.
        if fresh || (!record.spec.enabled && spec.enabled) {
            record.cursor = since.to_rfc3339();
        }
        record.spec = spec;
        if let Some(memory) = memory {
            record.memory = memory;
            record.memory_updated_at = Some(now());
        }
        if let Some(commit) = bundle_commit {
            record.bundle_commit = Some(commit);
            record.bundle_at = bundle_at;
        }
        let record = record.clone();
        drop(schedules);
        self.save_record(&record);
        Ok(())
    }

    pub(super) fn delete_schedule(&self, routine_id: &str) -> AppResult<()> {
        self.require_service()?;
        if !valid_id(routine_id) {
            return Err(AppError::validation("That routine ID is not valid."));
        }
        self.schedules().records.remove(routine_id);
        let _ = std::fs::remove_file(self.schedule_dir().join(format!("{routine_id}.json")));
        let _ = std::fs::remove_dir_all(self.state.run_dir(&bundle_key(routine_id)));
        Ok(())
    }

    pub(super) fn schedule_statuses(&self) -> (Vec<ScheduleStatus>, String) {
        let at = Local::now();
        let schedules = self.schedules();
        let live: HashMap<String, String> = self
            .runs()
            .values()
            .filter_map(|run| {
                let inner = run.lock();
                (inner.record.phase != Phase::Ended)
                    .then(|| {
                        inner
                            .record
                            .routine_id
                            .clone()
                            .map(|r| (r, inner.record.run_id.clone()))
                    })
                    .flatten()
            })
            .collect();
        let mut out: Vec<ScheduleStatus> = schedules
            .records
            .values()
            .map(|r| ScheduleStatus {
                spec: r.spec.clone(),
                memory: r.memory.clone(),
                bundle_commit: r.bundle_commit.clone(),
                bundle_at: r.bundle_at.clone(),
                next_fire: r
                    .spec
                    .enabled
                    .then(|| {
                        parse_time(&r.spec.time)
                            .and_then(|t| next_after(r.spec.days, t, at))
                            .map(|t| t.to_rfc3339())
                    })
                    .flatten(),
                live_run: live.get(&r.spec.routine_id).cloned(),
                credential: self.secrets.has_model(&r.spec.profile_id),
                results: r.results.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
        (out, at.to_rfc3339())
    }

    pub(super) fn ack_results(&self, routine_id: &str, ids: &[String]) -> AppResult<()> {
        self.require_service()?;
        let mut schedules = self.schedules();
        let Some(record) = schedules.records.get_mut(routine_id) else {
            return Ok(());
        };
        for result in &mut record.results {
            if ids.contains(&result.id) {
                result.acked = true;
            }
        }
        prune(&mut record.results);
        let record = record.clone();
        drop(schedules);
        self.save_record(&record);
        Ok(())
    }

    pub(super) async fn put_model_credential(
        &self,
        profile_id: &str,
        value: &str,
    ) -> AppResult<()> {
        self.require_service()?;
        self.secrets.put_model(profile_id, value).await
    }

    /// Look at every schedule: fire what is due, and follow the runs it fired.
    pub(super) async fn schedule_tick(self: &Arc<Self>) {
        if !self.service {
            return;
        }
        {
            let mut schedules = self.schedules();
            if schedules
                .last_look
                .is_some_and(|t| t.elapsed() < LOOK_EVERY)
            {
                return;
            }
            schedules.last_look = Some(Instant::now());
        }
        self.fire_due().await;
        self.follow_scheduled().await;
    }

    async fn fire_due(self: &Arc<Self>) {
        let at = Local::now();
        // Decide under the lock, act outside it.
        let mut to_fire: Vec<(String, DateTime<Local>)> = Vec::new();
        let mut changed: Vec<ScheduleRecord> = Vec::new();
        {
            let mut schedules = self.schedules();
            let persist_cursor = schedules
                .last_saved
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(60));
            if persist_cursor {
                schedules.last_saved = Some(Instant::now());
            }
            let live: Vec<String> = self
                .runs()
                .values()
                .filter_map(|run| {
                    let inner = run.lock();
                    (inner.record.phase != Phase::Ended)
                        .then(|| inner.record.routine_id.clone())
                        .flatten()
                })
                .collect();
            for record in schedules.records.values_mut() {
                if !record.spec.enabled {
                    continue;
                }
                let Some(time) = parse_time(&record.spec.time) else {
                    continue;
                };
                let after = parse_stamp(&record.cursor).unwrap_or(at);
                let slots = slots_in(record.spec.days, time, after, at);
                record.cursor = at.to_rfc3339();
                if slots.is_empty() {
                    if persist_cursor {
                        changed.push(record.clone());
                    }
                    continue;
                }
                let mut fired = live.contains(&record.spec.routine_id);
                let newest = slots.len() - 1;
                let mut missed = 0usize;
                for (i, slot) in slots.iter().enumerate() {
                    let age = at - *slot;
                    let on_time = age <= GRACE;
                    let late = record.spec.late_start && i == newest && !on_time;
                    if (on_time || late) && !fired {
                        fired = true;
                        to_fire.push((record.spec.routine_id.clone(), *slot));
                    } else if on_time || late {
                        record.results.insert(
                            0,
                            failure(
                                *slot,
                                None,
                                "The previous run was still going, so this one did not start.",
                            ),
                        );
                    } else if slots.len() - i <= MAX_MISSED {
                        missed += 1;
                        record.results.insert(0, missed_result(*slot));
                    }
                }
                let _ = missed;
                prune(&mut record.results);
                changed.push(record.clone());
            }
        }
        for record in &changed {
            self.save_record(record);
        }
        for (routine_id, slot) in to_fire {
            self.fire(&routine_id, slot).await;
        }
    }

    /// Start the routine's run for this slot, or record why it did not.
    async fn fire(self: &Arc<Self>, routine_id: &str, slot: DateTime<Local>) {
        let snapshot = {
            let schedules = self.schedules();
            schedules.records.get(routine_id).map(|r| {
                (
                    r.spec.clone(),
                    r.memory.clone(),
                    r.bundle_commit.clone(),
                    earlier_results(r),
                )
            })
        };
        let Some((spec, memory, commit, earlier)) = snapshot else {
            return;
        };
        let run_id = uuid::Uuid::new_v4().to_string();
        let started = self
            .begin_scheduled(&run_id, &spec, &memory, commit, &earlier)
            .await;
        if let Err(e) = started {
            tracing::warn!(routine = routine_id, error = %e, "a scheduled run did not start");
            self.record_result(
                routine_id,
                failure(slot, None, &format!("The run did not start: {}", e.message)),
            );
            return;
        }
        // The result is recorded when the run ends; remember which slot it was for.
        let mut schedules = self.schedules();
        schedules.driving.insert(run_id.clone(), Drive::default());
        schedules.slots.insert(run_id, slot.to_rfc3339());
    }

    async fn begin_scheduled(
        self: &Arc<Self>,
        run_id: &str,
        spec: &ScheduleSpec,
        memory: &str,
        commit: Option<String>,
        earlier: &[BriefingResult],
    ) -> AppResult<()> {
        let Some(commit) = commit else {
            return Err(conflict(
                "The host has no copy of the code yet: open Brainiac to send it.",
            ));
        };
        let source = self.state.input_bundle(&bundle_key(&spec.routine_id));
        if !source.is_file() {
            return Err(conflict(
                "The host's copy of the code is missing: open Brainiac to send it again.",
            ));
        }
        let value = self
            .secrets
            .get_model(&spec.profile_id)
            .await
            .map_err(|e| {
                AppError::dependency(format!(
                "The host cannot open the agent's key ({}). Approve the schedule again to send it.",
                e.message
            ))
            })?;
        let target = self.state.input_bundle(run_id);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &target)?;
        let start = StartRun {
            run_id: run_id.to_string(),
            attempt: 1,
            engine_socket: "/var/run/docker.sock".into(),
            image: spec.image.clone(),
            cpus: spec.cpus,
            memory_mib: spec.memory_mib,
            workspace_gib: spec.workspace_gib,
            time_limit_secs: spec.time_limit_secs,
            // Nobody is there to answer a question.
            permissions: RunPermissions::Act,
            start_commit: commit,
            bundle: target,
            prompt_id: format!("{run_id}-prompt-1"),
            prompt: routines::with_contract(&spec.prompt, memory, earlier),
            credential: Credential {
                key: spec.credential_key,
                value,
            },
            agent: spec.agent,
            provider: spec.provider,
            model: spec.model.clone(),
            known_concepts: None,
            secrets: Vec::new(),
            routine_id: Some(spec.routine_id.clone()),
            host_secrets: spec.host_secrets.clone(),
        };
        self.start(start).map(|_| ())
    }

    fn record_result(&self, routine_id: &str, result: ScheduledResult) {
        let mut schedules = self.schedules();
        let Some(record) = schedules.records.get_mut(routine_id) else {
            return;
        };
        if let Some(run_id) = &result.run_id {
            if record
                .results
                .iter()
                .any(|r| r.run_id.as_ref() == Some(run_id))
            {
                return;
            }
        }
        record.results.insert(0, result);
        prune(&mut record.results);
        let record = record.clone();
        drop(schedules);
        self.save_record(&record);
    }

    /// Read the briefing of each scheduled run that is idle, and note each
    /// one that ended without one.
    async fn follow_scheduled(self: &Arc<Self>) {
        struct Seen {
            run: Arc<super::Run>,
            routine_id: String,
            phase: Phase,
            activity: Activity,
            turn: u32,
            error: Option<String>,
            accepted_at: String,
        }
        let tagged: Vec<Seen> = self
            .runs()
            .values()
            .filter_map(|run| {
                let inner = run.lock();
                inner.record.routine_id.clone().map(|routine_id| Seen {
                    run: Arc::clone(run),
                    routine_id,
                    phase: inner.record.phase,
                    activity: inner.activity,
                    turn: inner.record.turn,
                    error: inner.record.error.clone(),
                    accepted_at: inner.record.accepted_at.clone(),
                })
            })
            .collect();
        for seen in tagged {
            let run_id = seen.run.id.clone();
            let recorded = self
                .schedules()
                .records
                .get(&seen.routine_id)
                .is_some_and(|r| {
                    r.results
                        .iter()
                        .any(|x| x.run_id.as_deref() == Some(run_id.as_str()))
                });
            if recorded {
                continue;
            }
            let slot = self
                .schedules()
                .slots
                .get(&run_id)
                .cloned()
                .unwrap_or_else(|| seen.accepted_at.clone());
            if seen.phase == Phase::Ended {
                let why = seen
                    .error
                    .clone()
                    .unwrap_or_else(|| "The run ended before it wrote a briefing.".to_string());
                self.record_result(
                    &seen.routine_id,
                    ScheduledResult {
                        id: uuid::Uuid::new_v4().to_string(),
                        slot,
                        run_id: Some(run_id),
                        state: ScheduledState::Failed,
                        started_at: seen.accepted_at,
                        finished_at: Some(now()),
                        draft: None,
                        error: Some(why),
                        acked: false,
                    },
                );
                continue;
            }
            if seen.activity == Activity::PlanLimit {
                self.record_result(
                    &seen.routine_id,
                    ScheduledResult {
                        id: uuid::Uuid::new_v4().to_string(),
                        slot,
                        run_id: Some(run_id.clone()),
                        state: ScheduledState::Failed,
                        started_at: seen.accepted_at,
                        finished_at: Some(now()),
                        draft: None,
                        error: Some(
                            "Plan limit reached. The host does not know when it resets; the next run will try again."
                                .into(),
                        ),
                        acked: false,
                    },
                );
                let _ = self.request_stop(&run_id, StopReason::Cancel);
                continue;
            }
            let idle = seen.phase == Phase::Running && seen.activity == Activity::Idle;
            let (checked, followed) = {
                let mut schedules = self.schedules();
                let drive = schedules.driving.entry(run_id.clone()).or_default();
                if seen.turn >= 2 {
                    drive.followed_up = true;
                }
                (drive.checked_turn, drive.followed_up)
            };
            if !idle || seen.turn <= checked {
                continue;
            }
            // A read that cannot be made now is tried again next time.
            let Ok(text) = self.read_result_file(&run_id, super::BRIEFING_PATH).await else {
                continue;
            };
            if let Some(drive) = self.schedules().driving.get_mut(&run_id) {
                drive.checked_turn = seen.turn;
            }
            let parsed = match text {
                None => Err(vec![
                    "There is no .brainiac/briefing.json: write it as the prompt asked."
                        .to_string(),
                ]),
                Some(text) => routines::parse(&text),
            };
            match parsed {
                Ok(draft) => {
                    self.take_briefing(&seen.routine_id, &run_id, &slot, &seen.accepted_at, draft);
                    if self.request_stop(&run_id, StopReason::Finish).is_err() {
                        let _ = self.request_stop(&run_id, StopReason::Cancel);
                    }
                }
                Err(problems) if !followed => {
                    if let Some(drive) = self.schedules().driving.get_mut(&run_id) {
                        drive.followed_up = true;
                    }
                    let command_id = format!("{run_id}-prompt-{}", seen.turn + 1);
                    let _ = self
                        .prompt(&run_id, &command_id, routines::follow_up(&problems))
                        .await;
                }
                Err(problems) => {
                    self.record_result(
                        &seen.routine_id,
                        ScheduledResult {
                            id: uuid::Uuid::new_v4().to_string(),
                            slot,
                            run_id: Some(run_id.clone()),
                            state: ScheduledState::Failed,
                            started_at: seen.accepted_at,
                            finished_at: Some(now()),
                            draft: None,
                            error: Some(format!(
                                "The agent did not write a usable briefing: {}",
                                problems.join(" ")
                            )),
                            acked: false,
                        },
                    );
                    let _ = self.request_stop(&run_id, StopReason::Cancel);
                }
            }
        }
    }

    /// Keep a checked briefing and the memory the agent returned.
    fn take_briefing(
        &self,
        routine_id: &str,
        run_id: &str,
        slot: &str,
        started_at: &str,
        draft: Draft,
    ) {
        let mut schedules = self.schedules();
        let Some(record) = schedules.records.get_mut(routine_id) else {
            return;
        };
        // The agent's memory, unless the user edited it after this run began.
        if let Some(memory) = &draft.memory {
            let edited_since = record
                .memory_updated_at
                .as_deref()
                .is_some_and(|t| t > started_at);
            if !edited_since {
                record.memory = memory.clone();
                record.memory_updated_at = Some(now());
            }
        }
        record.results.insert(
            0,
            ScheduledResult {
                id: uuid::Uuid::new_v4().to_string(),
                slot: slot.to_string(),
                run_id: Some(run_id.to_string()),
                state: ScheduledState::Ready,
                started_at: started_at.to_string(),
                finished_at: Some(now()),
                draft: Some(draft),
                error: None,
                acked: false,
            },
        );
        prune(&mut record.results);
        let record = record.clone();
        drop(schedules);
        self.save_record(&record);
    }
}

fn failure(slot: DateTime<Local>, run_id: Option<String>, why: &str) -> ScheduledResult {
    ScheduledResult {
        id: uuid::Uuid::new_v4().to_string(),
        slot: slot.to_rfc3339(),
        run_id,
        state: ScheduledState::Failed,
        started_at: now(),
        finished_at: Some(now()),
        draft: None,
        error: Some(why.to_string()),
        acked: false,
    }
}

fn missed_result(slot: DateTime<Local>) -> ScheduledResult {
    ScheduledResult {
        id: uuid::Uuid::new_v4().to_string(),
        slot: slot.to_rfc3339(),
        run_id: None,
        state: ScheduledState::Missed,
        started_at: now(),
        finished_at: Some(now()),
        draft: None,
        error: Some(format!(
            "The host was not running at {}, and this schedule does not start late.",
            slot.format("%H:%M on %a %-d %b")
        )),
        acked: false,
    }
}

/// Keep the newest results; one the Mac has not taken is never dropped.
fn prune(results: &mut Vec<ScheduledResult>) {
    let mut kept = 0usize;
    results.retain(|r| {
        if !r.acked {
            return true;
        }
        kept += 1;
        kept <= KEEP_RESULTS
    });
}

/// The ready briefings as the prompt shows them, newest first, as many as
/// the schedule asks for.
fn earlier_results(record: &ScheduleRecord) -> Vec<BriefingResult> {
    record
        .results
        .iter()
        .filter(|r| r.state == ScheduledState::Ready)
        .filter_map(|r| {
            let draft = r.draft.as_ref()?;
            Some(BriefingResult {
                id: r.id.clone(),
                routine_id: record.spec.routine_id.clone(),
                run_id: r.run_id.clone(),
                state: BriefingState::Ready,
                status: draft.status,
                summary: draft.summary.clone(),
                findings: draft.findings.clone(),
                proposals: draft.proposals.clone(),
                error: None,
                started_at: r.started_at.clone(),
                finished_at: r.finished_at.clone(),
                duration_secs: None,
            })
        })
        .take(record.spec.previous_results as usize)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .earliest()
            .unwrap()
    }

    #[test]
    fn a_time_is_hours_and_minutes() {
        assert!(parse_time("07:00").is_some());
        assert!(parse_time("7:00").is_none());
        assert!(parse_time("25:00").is_none());
        assert!(parse_time("07:60").is_none());
        assert!(parse_time("noon").is_none());
    }

    #[test]
    fn weekdays_skip_the_weekend_and_a_weekly_slot_is_one_day() {
        // 2026-10-12 is a Monday.
        let time = parse_time("07:00").unwrap();
        let from = local(2026, 10, 9, 12, 0); // Friday noon
        let until = local(2026, 10, 14, 12, 0); // Wednesday noon
        let slots = slots_in(ScheduleDays::Weekdays, time, from, until);
        let days: Vec<_> = slots.iter().map(|s| s.day()).collect();
        assert_eq!(days, [12, 13, 14]);
        let daily = slots_in(ScheduleDays::Daily, time, from, until);
        assert_eq!(daily.len(), 5);
        let friday = slots_in(ScheduleDays::Weekly(4), time, from, until);
        assert!(friday.is_empty());
        let next = next_after(ScheduleDays::Weekly(4), time, from).unwrap();
        assert_eq!(next.day(), 16);
    }

    #[test]
    fn a_slot_is_after_the_start_and_up_to_now() {
        let time = parse_time("07:00").unwrap();
        let at = local(2026, 10, 14, 7, 0);
        // Exactly at the start does not count; exactly now does.
        assert!(slots_in(ScheduleDays::Daily, time, at, at).is_empty());
        let before = local(2026, 10, 14, 6, 59);
        assert_eq!(slots_in(ScheduleDays::Daily, time, before, at).len(), 1);
    }

    #[test]
    fn unacknowledged_results_are_never_pruned() {
        let mut results: Vec<ScheduledResult> = (0..30)
            .map(|i| {
                let mut r = missed_result(local(2026, 10, 1, 7, 0));
                r.acked = i % 2 == 0;
                r
            })
            .collect();
        prune(&mut results);
        assert_eq!(results.iter().filter(|r| !r.acked).count(), 15);
        assert!(results.iter().filter(|r| r.acked).count() <= KEEP_RESULTS);
    }
}
