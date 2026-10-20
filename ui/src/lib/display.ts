import type { ResultItem } from "./bindings";

/**
 * Shortens a long path by cutting folders out of the middle, keeping the
 * drive/start and the closest folders: `C:\Users\…\Projects\site`.
 */
export function shortenPath(path: string, max = 56): string {
  if (path.length <= max) return path;
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(sep);
  if (parts.length < 3) return `…${path.slice(path.length - max + 1)}`;
  const head = parts[0] ?? "";
  const tail: string[] = [];
  let length = head.length + 2; // head + "\…"
  for (let i = parts.length - 1; i > 0; i--) {
    const part = parts[i] ?? "";
    if (length + part.length + 1 > max && tail.length > 0) break;
    tail.unshift(part);
    length += part.length + 1;
  }
  return tail.length === parts.length - 1 ? path : [head, "…", ...tail].join(sep);
}

/** Muted text shown after a result's title, if any. */
export function detailFor(item: ResultItem): string | null {
  if (item.kind === "file" || item.kind === "folder") return shortenPath(item.subtitle);
  // Setup rows (e.g. "Start Everything") explain themselves.
  if (item.kind === "command" && !item.id.startsWith("cmd:")) return item.subtitle;
  return null;
}
