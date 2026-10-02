import { MONTHS } from "./tasks";

/**
 * Note presentation helpers: paths inside the vault, edit times, and the
 * URLs Live Preview draws vault images from.
 */

/** The folder part of a vault path, or "" at the top. */
export function folderOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i === -1 ? "" : path.slice(0, i);
}

export function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** The file name without `.md`. */
export function stem(path: string): string {
  return fileName(path).replace(/\.md$/i, "");
}

/**
 * Edit times read as relative for the last seven days ("edited 2 hours
 * ago") and as dates after that ("edited 14 Sep") (SPEC.md, Main window).
 */
export function editedLabel(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return "edited just now";
  const m = Math.round(s / 60);
  if (m < 60) return `edited ${m} ${m === 1 ? "minute" : "minutes"} ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `edited ${h} ${h === 1 ? "hour" : "hours"} ago`;
  const d = Math.round(h / 24);
  if (d < 7) return `edited ${d} ${d === 1 ? "day" : "days"} ago`;
  const date = new Date(t);
  const year =
    date.getFullYear() === new Date(now).getFullYear()
      ? ""
      : ` ${date.getFullYear()}`;
  return `edited ${date.getDate()} ${MONTHS[date.getMonth()]}${year}`;
}

/** Resolve `.` and `..` in a vault path; null when it leaves the vault. */
export function normalizePath(path: string): string | null {
  const out: string[] = [];
  for (const part of path.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") {
      if (!out.length) return null;
      out.pop();
    } else out.push(part);
  }
  return out.length ? out.join("/") : null;
}

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

/**
 * The URL to draw an image from, for an image written in the note at
 * `notePath`: vault files through Brainiac's `vault:` scheme, nothing for a
 * web image, so opening a note makes no network request.
 */
export function vaultImageUrl(notePath: string, src: string): string | null {
  const target = src.trim().replace(/^<|>$/g, "").split(/[?#]/)[0];
  if (!target || SCHEME.test(target) || target.startsWith("//")) return null;
  let decoded: string;
  try {
    decoded = decodeURI(target);
  } catch {
    decoded = target;
  }
  const path = decoded.startsWith("/")
    ? normalizePath(decoded)
    : normalizePath(`${folderOf(notePath)}/${decoded}`);
  if (!path) return null;
  return `vault://localhost/${path.split("/").map(encodeURIComponent).join("/")}`;
}

/** A vault-relative path for an absolute path inside the vault, or null. */
export function insideVault(root: string, absolute: string): string | null {
  const base = root.endsWith("/") ? root : `${root}/`;
  return absolute.startsWith(base) ? absolute.slice(base.length) : null;
}
