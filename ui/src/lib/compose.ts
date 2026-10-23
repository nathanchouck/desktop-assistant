import type { Attachment, ProviderId } from "./bindings";
import { forcedProvider } from "./chat";

/** The question when you ask with attachments but type nothing (as in Rust). */
export const DEFAULT_QUESTION = "What can you tell me about this?";
/** More chips than this aren't kept. */
export const MAX_ATTACHMENTS = 12;
/** Chip labels are cut to this many characters. */
const LABEL_CHARS = 28;

/** `C:\Users\me\report.pdf` → `report.pdf` (either slash). */
export function fileName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const name = trimmed.split(/[\\/]/).pop();
  return name || path;
}

function cut(text: string, max: number): string {
  const chars = [...text];
  return chars.length <= max ? text : `${chars.slice(0, max - 1).join("")}…`;
}

/** The short text on an attachment's chip. */
export function chipLabel(a: Attachment): string {
  switch (a.type) {
    case "text":
      return `“${cut(a.text.replace(/\s+/g, " ").trim(), LABEL_CHARS)}”`;
    case "path":
      return cut(fileName(a.path), LABEL_CHARS);
    case "image":
      return `Picture ${a.width}×${a.height}`;
  }
}

/** What a chip's remove button says to screen readers. */
export function chipName(a: Attachment): string {
  switch (a.type) {
    case "text":
      return "the selected text";
    case "path":
      return fileName(a.path);
    case "image":
      return "the picture";
  }
}

/** The thumbnail key of an attached picture (served by the `icon` protocol). */
export function pictureKey(id: string): string {
  return `thumb/attach-${id}`;
}

function same(a: Attachment, b: Attachment): boolean {
  if (a.type === "text" && b.type === "text") return a.text === b.text;
  if (a.type === "path" && b.type === "path") return a.path === b.path;
  if (a.type === "image" && b.type === "image") return a.id === b.id;
  return false;
}

/** `existing` plus the ones in `added` it doesn't have yet, up to the limit. */
export function mergeAttachments(existing: Attachment[], added: Attachment[]): Attachment[] {
  const out = [...existing];
  for (const a of added) {
    if (out.length >= MAX_ATTACHMENTS) break;
    if (!out.some((b) => same(a, b))) out.push(a);
  }
  return out;
}

/**
 * The question typed with attachments, and the AI a `c `/`g ` prefix
 * picks. Nothing typed asks `DEFAULT_QUESTION`.
 */
export function composedQuestion(draft: string): { text: string; provider: ProviderId | null } {
  const forced = forcedProvider(draft);
  const text = (forced?.text ?? draft).trim();
  return { text: text || DEFAULT_QUESTION, provider: forced?.provider ?? null };
}

/** The first lines of selected text, for the compose view. */
export function textPreview(text: string, maxLines = 8, maxChars = 600): string {
  const lines = text.trim().split(/\r?\n/);
  const shown = lines.slice(0, maxLines).join("\n");
  if ([...shown].length > maxChars) return cut(shown, maxChars);
  return lines.length > maxLines ? `${shown}\n…` : shown;
}
