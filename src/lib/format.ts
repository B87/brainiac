export function relativeTime(
  iso: string | null | undefined,
  now = Date.now(),
): string {
  if (!iso) return "never";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.round((now - t) / 1000);
  if (s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(t).toLocaleDateString();
}

export function shortPath(p: string): string {
  const home = "/Users/";
  if (p.startsWith(home)) {
    const rest = p.slice(home.length);
    const slash = rest.indexOf("/");
    return slash === -1 ? "~" : `~${rest.slice(slash)}`;
  }
  return p;
}

/** Compact age for dense lists: "now", "3m", "2h", "5d", or a date. */
export function shortAgo(iso: string | null | undefined, now = Date.now()) {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const m = Math.floor((now - t) / 60_000);
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d`;
  return new Date(t).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
  });
}

/** Local date and time, e.g. "1 Oct 2026, 21:03". */
export function absoluteTime(iso: string | null | undefined): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  return new Date(t).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
