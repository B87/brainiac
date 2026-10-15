//! A run host's health as the window shows it (docs/design/scheduled-runs.md,
//! Machines): the controller's report and the engine's answer turned into
//! numbers and a list of what to do. Pure, so it is tested without a host.

use super::controller::health::HealthReport;
use super::controller::protocol::{EngineReport, PROTOCOL};
use crate::models::{AppError, HostHealth};

/// Below this much free disk, or this share of it, a run may not finish.
const LOW_DISK_KIB: u64 = 5 * 1024 * 1024;
const LOW_DISK_PERCENT: u64 = 10;
/// Below this share of memory available, a run may be killed.
const LOW_MEMORY_PERCENT: u64 = 10;

/// What a failed check shows: the host did not answer, and why in one
/// safe sentence.
pub fn unreachable(checked_at: String, problem: String) -> HostHealth {
    HostHealth {
        reachable: false,
        problem: Some(problem),
        checked_at,
        controller_version: None,
        controller_uptime_secs: None,
        live_runs: 0,
        host_uptime_secs: None,
        load: None,
        memory_total_kib: None,
        memory_available_kib: None,
        disk_total_kib: None,
        disk_free_kib: None,
        engine_name: None,
        engine_supported: None,
        engine_problem: None,
        warnings: Vec::new(),
    }
}

/// A controller from before Health answers that it did not understand the
/// request: the host is up and needs the upgrade, which is not an outage.
pub fn not_understood(e: &AppError) -> bool {
    e.message.contains("did not understand the request")
}

/// The host answered, but its controller predates Health.
pub fn needs_upgrade(checked_at: String) -> HostHealth {
    let mut health = unreachable(checked_at, String::new());
    health.reachable = true;
    health.problem = None;
    health.warnings.push(
        "This host's run controller is older than Brainiac, so it cannot report its health. Upgrade it in Settings → Agents once no run is live there."
            .into(),
    );
    health
}

/// The report and the engine's answer (`None` when the engine did not answer).
pub fn from_report(
    report: HealthReport,
    engine: Option<EngineReport>,
    checked_at: String,
) -> HostHealth {
    let mut warnings = Vec::new();
    if report.protocol != PROTOCOL || report.controller_version != env!("CARGO_PKG_VERSION") {
        warnings.push(format!(
            "The controller is version {}, and this app is {}. Upgrade the host in Settings → Agents.",
            report.controller_version,
            env!("CARGO_PKG_VERSION")
        ));
    }
    if let (Some(total), Some(free)) = (report.disk_total_kib, report.disk_free_kib) {
        if total > 0 && (free < LOW_DISK_KIB || free * 100 / total < LOW_DISK_PERCENT) {
            warnings.push(format!(
                "The disk holding the controller's data is almost full ({} free). Free space before the next run.",
                gib(free)
            ));
        }
    }
    if let (Some(total), Some(free)) = (report.memory_total_kib, report.memory_available_kib) {
        if total > 0 && free * 100 / total < LOW_MEMORY_PERCENT {
            warnings.push(format!(
                "Little memory is available ({} of {}). A run may be stopped by the host.",
                gib(free),
                gib(total)
            ));
        }
    }
    match &engine {
        None => warnings.push(
            "The Docker engine did not answer. Start it on the host, then check again.".into(),
        ),
        Some(e) if !e.supported => warnings.push(
            e.problem
                .clone()
                .unwrap_or_else(|| "This engine cannot take a run.".into()),
        ),
        Some(_) => {}
    }
    HostHealth {
        reachable: true,
        problem: None,
        checked_at,
        controller_version: Some(report.controller_version),
        controller_uptime_secs: Some(report.controller_uptime_secs),
        live_runs: report.live_runs,
        host_uptime_secs: report.host_uptime_secs,
        load: report.load.map(|l| l.to_vec()),
        memory_total_kib: report.memory_total_kib,
        memory_available_kib: report.memory_available_kib,
        disk_total_kib: report.disk_total_kib,
        disk_free_kib: report.disk_free_kib,
        engine_name: engine.as_ref().map(|e| e.name.clone()),
        engine_supported: engine.as_ref().map(|e| e.supported),
        engine_problem: engine.and_then(|e| e.problem),
        warnings,
    }
}

fn gib(kib: u64) -> String {
    format!("{:.1} GiB", kib as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> HealthReport {
        HealthReport {
            controller_version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL,
            controller_uptime_secs: 60,
            live_runs: 1,
            host_uptime_secs: Some(1000),
            load: Some([0.1, 0.2, 0.3]),
            memory_total_kib: Some(16 * 1024 * 1024),
            memory_available_kib: Some(8 * 1024 * 1024),
            disk_total_kib: Some(500 * 1024 * 1024),
            disk_free_kib: Some(300 * 1024 * 1024),
        }
    }

    fn engine(supported: bool) -> EngineReport {
        EngineReport {
            name: "Docker Engine".into(),
            supported,
            problem: (!supported)
                .then(|| "Runs on a remote host need a Linux Docker engine.".into()),
            loop_devices: false,
        }
    }

    #[test]
    fn a_healthy_host_has_no_warnings() {
        let h = from_report(report(), Some(engine(true)), "now".into());
        assert!(h.reachable);
        assert!(h.warnings.is_empty(), "{:?}", h.warnings);
        assert_eq!(h.engine_name.as_deref(), Some("Docker Engine"));
        assert_eq!(h.load, Some(vec![0.1, 0.2, 0.3]));
    }

    #[test]
    fn names_each_problem_with_what_to_do() {
        let mut r = report();
        r.disk_free_kib = Some(1024 * 1024);
        r.memory_available_kib = Some(1024 * 1024);
        r.controller_version = "0.0.1".into();
        let h = from_report(r, None, "now".into());
        assert_eq!(h.warnings.len(), 4, "{:?}", h.warnings);
        assert!(h.warnings[0].contains("Upgrade the host"));
        assert!(h.warnings[1].contains("almost full"));
        assert!(h.warnings[2].contains("memory"));
        assert!(h.warnings[3].contains("engine did not answer"));
    }

    #[test]
    fn an_unsupported_engine_says_why() {
        let h = from_report(report(), Some(engine(false)), "now".into());
        assert_eq!(h.engine_supported, Some(false));
        assert!(h.warnings[0].contains("Linux Docker engine"));
    }

    #[test]
    fn numbers_a_host_cannot_give_stay_absent() {
        let mut r = report();
        r.disk_total_kib = None;
        r.disk_free_kib = None;
        r.memory_total_kib = None;
        r.memory_available_kib = None;
        let h = from_report(r, Some(engine(true)), "now".into());
        assert!(h.warnings.is_empty());
        assert_eq!(h.disk_free_kib, None);
    }

    #[test]
    fn an_older_controller_is_up_and_needs_an_upgrade() {
        let refused = AppError::validation("The run controller did not understand the request.");
        assert!(not_understood(&refused));
        assert!(!not_understood(&AppError::validation("Something else.")));
        let h = needs_upgrade("now".into());
        assert!(h.reachable);
        assert_eq!(h.problem, None);
        assert!(h.warnings[0].contains("Upgrade it"));
    }

    #[test]
    fn an_unreachable_host_has_only_the_reason() {
        let h = unreachable("now".into(), "The host did not answer.".into());
        assert!(!h.reachable);
        assert_eq!(h.problem.as_deref(), Some("The host did not answer."));
        assert!(h.warnings.is_empty());
    }
}
