/**
 * A run host's Health (docs/design/scheduled-runs.md, Machines): how the
 * numbers read. A host that cannot give a number shows a dash.
 */
import type { HostHealth } from "./generated/HostHealth";

const DASH = "—";

/** "3 d 4 h", "2 h 5 min", "7 min", "40 s". Two units at most. */
export function uptimeLabel(secs: number | null | undefined): string {
  if (secs == null || secs < 0) return DASH;
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d} d ${h} h`;
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min`;
  return `${Math.floor(secs)} s`;
}

/** Kibibytes as "512 MiB" or "3.4 GiB". */
export function sizeLabel(kib: number | null | undefined): string {
  if (kib == null || kib < 0) return DASH;
  const mib = kib / 1024;
  if (mib < 1024) return `${Math.round(mib)} MiB`;
  const gib = mib / 1024;
  if (gib < 1024) return `${gib.toFixed(1)} GiB`;
  return `${(gib / 1024).toFixed(1)} TiB`;
}

/** "8.0 GiB free of 16.0 GiB (50%)": free space and its share, rounded down as the warnings are. */
export function freeLabel(
  free: number | null | undefined,
  total: number | null | undefined,
): string {
  if (free == null || total == null || total <= 0) return DASH;
  return `${sizeLabel(free)} free of ${sizeLabel(total)} (${Math.floor((free / total) * 100)}%)`;
}

/** "0.10 · 0.20 · 0.30" for the 1, 5, and 15 minute averages. */
export function loadLabel(load: number[] | null | undefined): string {
  if (!load || load.length < 3) return DASH;
  return load
    .slice(0, 3)
    .map((n) => n.toFixed(2))
    .join(" · ");
}

export type HealthTone = "ok" | "warn" | "bad";

/** The host as a whole: down, needing attention, or fine. */
export function healthTone(h: HostHealth): HealthTone {
  if (!h.reachable) return "bad";
  return h.warnings.length > 0 ? "warn" : "ok";
}

/** One line for the header of the section. */
export function healthSummary(h: HostHealth): string {
  if (!h.reachable) return h.problem ?? "The host did not answer.";
  if (h.warnings.length === 0) return "Reachable, and nothing needs attention";
  return h.warnings.length === 1
    ? "1 thing needs attention"
    : `${h.warnings.length} things need attention`;
}
