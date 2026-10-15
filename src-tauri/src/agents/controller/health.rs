//! What the controller tells about its own host (docs/design/scheduled-runs.md,
//! Machines): a fixed set of numbers it reads itself. It is not a shell and
//! not a relay to the engine. On a system without `/proc` (this Mac) the
//! host's numbers are `None` and the client shows a dash.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// When this process first asked. `serve` asks once at startup, so later
/// calls measure the controller's own uptime.
// `OnceLock` holds a value that is set once and then only read, here from
// any thread without a lock around every read.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Remember the start time; later calls keep the first.
pub fn mark_started() {
    let _ = STARTED.get_or_init(Instant::now);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthReport {
    pub controller_version: String,
    pub protocol: u32,
    pub controller_uptime_secs: u64,
    pub live_runs: u32,
    pub host_uptime_secs: Option<u64>,
    /// The 1, 5, and 15 minute load averages.
    pub load: Option<[f64; 3]>,
    pub memory_total_kib: Option<u64>,
    pub memory_available_kib: Option<u64>,
    /// The volume holding the controller's state directory, which is also
    /// where run workspaces and the engine's data are expected to live.
    pub disk_total_kib: Option<u64>,
    pub disk_free_kib: Option<u64>,
}

/// Read the report. `state` is the controller's state directory.
pub async fn collect(state: &Path, live_runs: u32) -> HealthReport {
    mark_started();
    let (memory_total_kib, memory_available_kib) =
        match std::fs::read_to_string("/proc/meminfo").ok().as_deref() {
            Some(text) => parse_meminfo(text),
            None => (None, None),
        };
    let (disk_total_kib, disk_free_kib) = disk(state).await;
    HealthReport {
        controller_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol: super::protocol::PROTOCOL,
        controller_uptime_secs: STARTED.get().map_or(0, |t| t.elapsed().as_secs()),
        live_runs,
        host_uptime_secs: std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|t| parse_uptime(&t)),
        load: std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|t| parse_loadavg(&t)),
        memory_total_kib,
        memory_available_kib,
        disk_total_kib,
        disk_free_kib,
    }
}

/// How long `df` may take. A stale network mount can leave it stuck.
const DF_TIMEOUT: Duration = Duration::from_secs(5);

/// `df` with an argument array, on the state directory. It runs as a
/// child process the runtime can wait on, and is killed when dropped, so a
/// hung mount costs five seconds and no thread. A failure is `None`s.
async fn disk(path: &Path) -> (Option<u64>, Option<u64>) {
    let mut command = tokio::process::Command::new("df");
    command.arg("-Pk").arg(path).kill_on_drop(true);
    let Ok(Ok(out)) = tokio::time::timeout(DF_TIMEOUT, command.output()).await else {
        return (None, None);
    };
    if !out.status.success() {
        return (None, None);
    }
    parse_df(&String::from_utf8_lossy(&out.stdout))
}

/// The first field of `/proc/uptime`, in whole seconds.
pub fn parse_uptime(text: &str) -> Option<u64> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then_some(secs as u64)
}

/// The first three fields of `/proc/loadavg`.
pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut fields = text.split_whitespace();
    let mut load = [0.0; 3];
    for slot in &mut load {
        let value: f64 = fields.next()?.parse().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        *slot = value;
    }
    Some(load)
}

/// `MemTotal` and `MemAvailable` of `/proc/meminfo`, in KiB.
pub fn parse_meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
    };
    (field("MemTotal"), field("MemAvailable"))
}

/// Total and available KiB from the data line of `df -Pk`: name, total,
/// used, available, capacity (ending in `%`), mount point. The name and the
/// mount point may hold spaces, so the numbers are found from the capacity.
pub fn parse_df(text: &str) -> (Option<u64>, Option<u64>) {
    let Some(line) = text.lines().nth(1) else {
        return (None, None);
    };
    let fields: Vec<&str> = line.split_whitespace().collect();
    for i in 3..fields.len() {
        if !fields[i].ends_with('%') {
            continue;
        }
        let number = |s: &str| s.parse::<u64>().ok();
        if let (Some(total), Some(_used), Some(free)) = (
            number(fields[i - 3]),
            number(fields[i - 2]),
            number(fields[i - 1]),
        ) {
            return (Some(total), Some(free));
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_uptime_and_load() {
        assert_eq!(parse_uptime("351735.47 1000.2\n"), Some(351_735));
        assert_eq!(parse_uptime("x"), None);
        assert_eq!(parse_uptime("-1 2"), None);
        assert_eq!(
            parse_loadavg("0.20 0.18 0.12 1/405 9999\n"),
            Some([0.20, 0.18, 0.12])
        );
        assert_eq!(parse_loadavg("0.20 0.18"), None);
        assert_eq!(parse_loadavg("nan 0 0"), None);
    }

    #[test]
    fn reads_memory() {
        let text =
            "MemTotal:       16384000 kB\nMemFree:         100 kB\nMemAvailable:    8192000 kB\n";
        assert_eq!(parse_meminfo(text), (Some(16_384_000), Some(8_192_000)));
        assert_eq!(parse_meminfo("Other: 1 kB"), (None, None));
    }

    #[test]
    fn reads_disk() {
        let text = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                    /dev/sda1        102400000  50000000  52400000      49% /\n";
        assert_eq!(parse_df(text), (Some(102_400_000), Some(52_400_000)));
        // A name or a mount point with spaces does not shift the columns.
        let spaced = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                      //nas/my share 2000 500 1500 25% /mnt/my share\n";
        assert_eq!(parse_df(spaced), (Some(2000), Some(1500)));
        assert_eq!(parse_df("Filesystem\n"), (None, None));
        assert_eq!(parse_df(""), (None, None));
    }

    #[tokio::test]
    async fn the_report_has_this_controllers_version() {
        let report = collect(Path::new("."), 2).await;
        assert_eq!(report.live_runs, 2);
        assert_eq!(report.protocol, super::super::protocol::PROTOCOL);
        assert!(!report.controller_version.is_empty());
    }
}
