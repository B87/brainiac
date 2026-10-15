import { useCallback, useEffect, useRef, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  freeLabel,
  healthSummary,
  healthTone,
  loadLabel,
  uptimeLabel,
} from "../lib/hostHealth";
import { type AgentHost, errorMessage, type HostHealth, ipc } from "../lib/ipc";
import { HostPill } from "./HostJobView";
import { Group, Hint } from "./SettingsPanes";

/** How often the numbers are read while this section is on screen. */
const REFRESH_MS = 30_000;

const TONE = { ok: "ready", warn: "attention", bad: "failed" } as const;

/**
 * Health of a run host (docs/design/scheduled-runs.md, Machines): a live
 * view, not monitoring. It reads the controller's numbers when the section
 * opens and then every 30 seconds while the window is visible; nothing is
 * stored, and no alert is sent.
 */
export default function HostHealthGroup({ host }: { host: AgentHost }) {
  const [health, setHealth] = useState<HostHealth | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // A read still running is not started again by the timer or a click.
  const inFlight = useRef(false);

  const check = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      setHealth(await ipc.getAgentHostHealth(host.id));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }, [host.id]);

  useEffect(() => {
    void check();
    const refresh = () => {
      if (document.visibilityState === "visible") void check();
    };
    const timer = setInterval(refresh, REFRESH_MS);
    // Coming back to the window reads at once, not up to 30 seconds later.
    document.addEventListener("visibilitychange", refresh);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [check]);

  const tone = health ? healthTone(health) : null;
  return (
    <Group label="Health">
      <div className="settings-group flex flex-col">
        <div className="settings-row">
          <span className="min-w-0 flex-1">
            {health ? healthSummary(health) : "Checking…"}
          </span>
          {health && tone && (
            <HostPill
              label={
                tone === "ok"
                  ? "Healthy"
                  : tone === "warn"
                    ? "Needs attention"
                    : "Not reachable"
              }
              tone={TONE[tone]}
            />
          )}
          <button
            type="button"
            className="btn btn-sm"
            disabled={busy}
            onClick={() => void check()}
          >
            {busy ? "Checking…" : "Check Now"}
          </button>
        </div>
        {health?.warnings.map((w) => (
          <div key={w} className="settings-row text-dirty" role="alert">
            {w}
          </div>
        ))}
        {health?.reachable && (
          <dl className="m-0 flex flex-col">
            <Row
              label="Host up for"
              value={uptimeLabel(health.host_uptime_secs)}
            />
            <Row label="Load (1, 5, 15 min)" value={loadLabel(health.load)} />
            <Row
              label="Memory"
              value={freeLabel(
                health.memory_available_kib,
                health.memory_total_kib,
              )}
            />
            <Row
              label="Disk (controller's data)"
              value={freeLabel(health.disk_free_kib, health.disk_total_kib)}
            />
            <Row
              label="Docker engine"
              value={
                health.engine_name
                  ? `${health.engine_name}${health.engine_supported === false ? " (cannot take a run)" : ""}`
                  : "Not answering"
              }
            />
            <Row
              label="Controller"
              value={`${health.controller_version ?? "—"} · up ${uptimeLabel(health.controller_uptime_secs)} · ${health.live_runs === 1 ? "1 run" : `${health.live_runs} runs`} now`}
            />
          </dl>
        )}
        {error && (
          <div className="settings-row text-dirty" role="alert">
            {error}
          </div>
        )}
      </div>
      <Hint>
        {health ? `Checked ${relativeTime(health.checked_at)}. ` : ""}A live
        view, not monitoring: nothing is stored and no alert is sent.
      </Hint>
    </Group>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-row">
      <dt className="min-w-0 flex-1 text-fg-2">{label}</dt>
      <dd className="m-0 tabular">{value}</dd>
    </div>
  );
}
