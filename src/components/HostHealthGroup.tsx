import { useCallback, useEffect, useRef, useState } from "react";
import { relativeTime } from "../lib/format";
import {
  freeLabel,
  healthSummary,
  healthTone,
  loadLabel,
  uptimeLabel,
} from "../lib/hostHealth";
import type { HostTone } from "../lib/hostJobs";
import { errorMessage, type HostHealth, ipc } from "../lib/ipc";
import { HostPill } from "./HostJobView";
import { Card, Hint } from "./SettingsPanes";

/** How often the numbers are read while the page is on screen. */
const REFRESH_MS = 30_000;

const TONE = { ok: "ready", warn: "attention", bad: "failed" } as const;

export type HostHealthState = {
  health: HostHealth | null;
  error: string | null;
  busy: boolean;
  /** Reads at once (Check Now). */
  check: () => Promise<void>;
};

/**
 * Health of a run host (docs/design/scheduled-runs.md, Machines): a live
 * view, not monitoring. It reads the controller's numbers when the page
 * opens and then every 30 seconds while the window is visible and `enabled`;
 * nothing is stored, and no alert is sent.
 */
export function useHostHealth(
  hostId: string,
  enabled: boolean,
): HostHealthState {
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
      setHealth(await ipc.getAgentHostHealth(hostId));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }, [hostId]);

  useEffect(() => {
    setHealth(null);
    setError(null);
    if (!enabled) return;
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
  }, [check, enabled]);

  return { health, error, busy, check };
}

/** The host's page header pill, once its health was read. */
export function healthPill(h: HostHealth): { label: string; tone: HostTone } {
  const tone = healthTone(h);
  return tone === "ok"
    ? { label: `Reachable · ${relativeTime(h.checked_at)}`, tone: "ready" }
    : tone === "warn"
      ? { label: "Needs attention", tone: "attention" }
      : { label: "Not reachable", tone: "failed" };
}

/**
 * The Health card: load, memory, the disk that holds the controller's data,
 * the engine, and the runs on the host now, with what needs attention.
 */
export default function HostHealthCard({
  state,
  className,
}: {
  state: HostHealthState;
  className?: string;
}) {
  const { health, error } = state;
  const tone = health ? healthTone(health) : null;
  return (
    <Card
      title="Health"
      className={className}
      actions={
        health &&
        tone && (
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
        )
      }
    >
      <div className="settings-row">
        <Hint>{health ? healthSummary(health) : "Checking…"}</Hint>
      </div>
      {health?.warnings.map((w) => (
        <div
          key={w}
          className="settings-row text-[12.5px] text-dirty"
          role="alert"
        >
          {w}
        </div>
      ))}
      {health?.reachable && (
        <dl className="m-0 flex flex-col">
          <Row label="Load" value={loadLabel(health.load)} />
          <Row
            label="Memory"
            value={freeLabel(
              health.memory_available_kib,
              health.memory_total_kib,
            )}
          />
          <Row
            label="Disk"
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
          <Row label="Runs live" value={String(health.live_runs)} />
          <Row
            label="Host up"
            value={`up ${uptimeLabel(health.host_uptime_secs)}`}
          />
          <Row
            label="Run controller"
            value={`${health.controller_version ?? "—"} · up ${uptimeLabel(health.controller_uptime_secs)}`}
          />
        </dl>
      )}
      {error && (
        <div className="settings-row text-[12.5px] text-dirty" role="alert">
          {error}
        </div>
      )}
      <div className="settings-row">
        <Hint>
          {health ? `Checked ${relativeTime(health.checked_at)}. ` : ""}A live
          view, not monitoring: nothing is stored and no alert is sent.
        </Hint>
      </div>
    </Card>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="settings-row min-h-10 py-2">
      <dt className="min-w-0 flex-1 text-[12.5px] text-fg-2">{label}</dt>
      <dd className="m-0 tabular text-[12.5px]">{value}</dd>
    </div>
  );
}
