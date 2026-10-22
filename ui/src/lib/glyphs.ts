/**
 * Built-in symbols for results without a real icon. A result asks for one
 * with `icon: "glyph:<name>"` (Rust: `search::glyph("<name>")`).
 *
 * Each is the inside of a 24×24 SVG drawn with strokes (no fills), in the
 * style of the kind glyphs in `ItemIcon.svelte`. Static markup only: never
 * build these from result text.
 */
export const GLYPHS: Record<string, string> = {
  // Launcher kinds
  app: '<rect x="4" y="4" width="7" height="7" rx="2"/><rect x="13" y="4" width="7" height="7" rx="2"/><rect x="4" y="13" width="7" height="7" rx="2"/><rect x="13" y="13" width="7" height="7" rx="2"/>',
  command: '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M7.5 10l3 2.5-3 2.5M12.5 15.5h4"/>',
  folder: '<path d="M3.5 7.5a2 2 0 012-2h4l2 2h7a2 2 0 012 2v8a2 2 0 01-2 2h-13a2 2 0 01-2-2z"/>',
  file: '<path d="M6.5 3.5h7l4 4v13h-11z M13.5 3.5v4h4"/>',
  sparkle: '<path d="M12 3l1.8 4.9L19 9.7l-5.2 1.8L12 16.4l-1.8-4.9L5 9.7l5.2-1.8z"/><path d="M18.5 15l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z"/>',
  chatgpt: '<path d="M12 3.5l7.4 4.25v8.5L12 20.5l-7.4-4.25v-8.5z"/><path d="M12 8.5l3 1.75v3.5L12 15.5l-3-1.75v-3.5z"/>',
  settings: '<line x1="4" y1="7" x2="20" y2="7"/><circle cx="9" cy="7" r="2.4"/><line x1="4" y1="17" x2="20" y2="17"/><circle cx="15" cy="17" r="2.4"/>',

  // Answers and text
  calculator: '<rect x="5.5" y="3.5" width="13" height="17" rx="2.5"/><path d="M8.5 7.5h7M8.5 12h1M11.5 12h1M14.5 12h1M8.5 16h1M11.5 16h1M14.5 16h1"/>',
  convert: '<path d="M5 8.5h13l-3.5-3.5M19 15.5H6l3.5 3.5"/>',
  currency: '<circle cx="12" cy="12" r="8.5"/><path d="M14.8 9.2c-.5-1-1.6-1.6-2.8-1.6-1.6 0-2.8.9-2.8 2.1 0 2.9 5.8 1.5 5.8 4.4 0 1.2-1.3 2.2-3 2.2-1.3 0-2.4-.6-2.9-1.6M12 6v1.6M12 16.4V18"/>',
  clipboard: '<rect x="5.5" y="5" width="13" height="16" rx="2"/><path d="M9 5V4a1 1 0 011-1h4a1 1 0 011 1v1M9 10.5h6M9 14h6M9 17.5h4"/>',
  snippet: '<path d="M5 6.5h14M5 11h14M5 15.5h9"/><path d="M17 15l2.5 2.5L17 20"/>',
  emoji: '<circle cx="12" cy="12" r="8.5"/><path d="M8.5 14.2c.9 1.3 2.1 2 3.5 2s2.6-.7 3.5-2"/><path d="M9.2 9.8h.01M14.8 9.8h.01" stroke-width="2.4"/>',
  image: '<rect x="3.5" y="5" width="17" height="14" rx="2"/><circle cx="9" cy="10" r="1.6"/><path d="M4 17.5l4.8-4.3 3.4 3 3.3-2.7 4.5 4"/>',
  pin: '<path d="M9 4.5h6l-1 5 3 3H7l3-3z M12 12.5V20"/>',

  // Web
  link: '<path d="M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.4 2.4 3.6 5.2 3.6 8.5s-1.2 6.1-3.6 8.5c-2.4-2.4-3.6-5.2-3.6-8.5S9.6 5.9 12 3.5z"/>',
  search: '<circle cx="10.5" cy="10.5" r="6"/><path d="M15 15l5 5"/>',

  // Windows and processes
  window: '<rect x="3.5" y="5" width="17" height="14" rx="2"/><path d="M3.5 9h17"/>',
  "layout-left": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="5.5" y="7" width="6" height="10" rx="1"/>',
  "layout-right": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="12.5" y="7" width="6" height="10" rx="1"/>',
  "layout-top": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="5.5" y="7" width="13" height="4.5" rx="1"/>',
  "layout-bottom": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="5.5" y="12.5" width="13" height="4.5" rx="1"/>',
  "layout-thirds": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><path d="M9.2 5v14M14.8 5v14"/>',
  "layout-center": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="7.5" y="8" width="9" height="8" rx="1"/>',
  "layout-left-third": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="5.5" y="7" width="3.8" height="10" rx="1"/>',
  "layout-center-third": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="10.1" y="7" width="3.8" height="10" rx="1"/>',
  "layout-right-third": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="14.7" y="7" width="3.8" height="10" rx="1"/>',
  "layout-left-two-thirds": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="5.5" y="7" width="8.4" height="10" rx="1"/>',
  "layout-right-two-thirds": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="10.1" y="7" width="8.4" height="10" rx="1"/>',
  "almost-maximize": '<rect x="3.5" y="5" width="17" height="14" rx="2"/><rect x="6" y="7.5" width="12" height="9" rx="1"/>',
  restore: '<rect x="3.5" y="8.5" width="12" height="11" rx="2"/><path d="M8 8.5V6.5a2 2 0 012-2h8.5a2 2 0 012 2V14a2 2 0 01-2 2h-3"/>',
  maximize: '<rect x="3.5" y="5" width="17" height="14" rx="2"/><path d="M8 13V9h4M16 11v4h-4"/>',
  monitor: '<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M9 20.5h6M12 16.5v4"/>',
  process: '<rect x="7" y="7" width="10" height="10" rx="1.5"/><path d="M10 3.5v3.5M14 3.5v3.5M10 17v3.5M14 17v3.5M3.5 10H7M3.5 14H7M17 10h3.5M17 14h3.5"/>',
  "x-circle": '<circle cx="12" cy="12" r="8.5"/><path d="M9 9l6 6M15 9l-6 6"/>',

  // System
  power: '<path d="M12 3.5v8"/><path d="M7.2 6.8a7.5 7.5 0 109.6 0"/>',
  restart: '<path d="M19.5 12a7.5 7.5 0 11-2.2-5.3"/><path d="M19.5 4.5v4h-4"/>',
  lock: '<rect x="5.5" y="10.5" width="13" height="10" rx="2"/><path d="M8.5 10.5V8a3.5 3.5 0 017 0v2.5"/>',
  sleep: '<path d="M19 14.5A7.5 7.5 0 019.5 5a7.5 7.5 0 109.5 9.5z"/>',
  hibernate: '<path d="M17 14.5A7 7 0 019.5 6a7 7 0 107.5 8.5z"/><path d="M15 3.5h4.5l-4.5 4.5h4.5"/>',
  logout: '<path d="M14 4.5H6.5a2 2 0 00-2 2v11a2 2 0 002 2H14"/><path d="M10.5 12h9.5M16.5 8.5L20 12l-3.5 3.5"/>',
  trash: '<path d="M4.5 7h15M9.5 7V5a1 1 0 011-1h3a1 1 0 011 1v2M6.5 7l1 12.5a1.5 1.5 0 001.5 1.5h6a1.5 1.5 0 001.5-1.5l1-12.5"/>',
  theme: '<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5a8.5 8.5 0 010 17z"/>',
  volume: '<path d="M4.5 9.5h3l4-3.5v12l-4-3.5h-3z"/><path d="M15 9a4 4 0 010 6M17.5 6.5a7.5 7.5 0 010 11"/>',
  mute: '<path d="M4.5 9.5h3l4-3.5v12l-4-3.5h-3z"/><path d="M15.5 9.5l5 5M20.5 9.5l-5 5"/>',
  wifi: '<path d="M3.5 9a12 12 0 0117 0M6.5 12.2a7.5 7.5 0 0111 0M9.4 15.3a3.4 3.4 0 015.2 0"/><path d="M12 18.6h.01" stroke-width="2.4"/>',
  bluetooth: '<path d="M7 7.5l10 9-5 4.5V3l5 4.5-10 9"/>',
  bell: '<path d="M6 16.5V11a6 6 0 0112 0v5.5l1.5 1.5h-15z"/><path d="M10 20.5a2 2 0 004 0"/>',
  keyboard: '<rect x="3" y="6.5" width="18" height="11" rx="2"/><path d="M7 10h.01M10 10h.01M13 10h.01M16 10h.01M8 14h8" stroke-width="1.8"/>',

  // Media
  play: '<path d="M8 5.5v13l10.5-6.5z"/>',
  pause: '<path d="M8.5 5.5v13M15.5 5.5v13"/>',
  "play-pause": '<path d="M5 6v12l7-6z"/><path d="M15 6v12M19 6v12"/>',
  next: '<path d="M6 6v12l8-6z"/><path d="M17.5 6v12"/>',
  previous: '<path d="M18 6v12l-8-6z"/><path d="M6.5 6v12"/>',
  music: '<path d="M9 17.5V6l10-2v11.5"/><circle cx="6.5" cy="17.5" r="2.5"/><circle cx="16.5" cy="15.5" r="2.5"/>',

  // Tools
  palette: '<path d="M12 3.5a8.5 8.5 0 000 17c1.1 0 1.8-.8 1.8-1.7 0-1.4-1.3-1.8-1.3-3 0-1 .8-1.6 1.8-1.6h2.2a4 4 0 004-4c0-3.7-3.8-6.7-8.5-6.7z"/><path d="M7.8 11.5h.01M10 7.8h.01M14.2 7.8h.01" stroke-width="2.4"/>',
  eyedropper: '<path d="M14.5 5.5l4 4M13 7l4 4-8.5 8.5H4.5v-4z"/><path d="M15.5 4.5a2.1 2.1 0 013 3l-1.5 1.5-3-3z"/>',
  screenshot: '<path d="M4 8V5.5A1.5 1.5 0 015.5 4H8M16 4h2.5A1.5 1.5 0 0120 5.5V8M20 16v2.5a1.5 1.5 0 01-1.5 1.5H16M8 20H5.5A1.5 1.5 0 014 18.5V16"/><rect x="8" y="8" width="8" height="8" rx="1"/>',
  mic: '<rect x="9" y="3.5" width="6" height="11" rx="3"/><path d="M5.5 11.5a6.5 6.5 0 0013 0M12 18v2.5"/>',
  history: '<path d="M4.5 12a7.5 7.5 0 102.2-5.3"/><path d="M4.5 4.5v3.2h3.2M12 8v4.2l3 1.8"/>',
  chat: '<path d="M5 5.5h14a1.5 1.5 0 011.5 1.5v8a1.5 1.5 0 01-1.5 1.5h-7l-4.5 3.5v-3.5H5A1.5 1.5 0 013.5 15V7A1.5 1.5 0 015 5.5z"/>',
  script: '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M7.5 9.5l2.5 2.5-2.5 2.5M12.5 14.5h4"/>',
  update: '<path d="M12 4v11M7.5 10.5L12 15l4.5-4.5"/><path d="M5 19.5h14"/>',
  calendar: '<rect x="4" y="5.5" width="16" height="14.5" rx="2"/><path d="M4 10h16M8.5 3.5v4M15.5 3.5v4"/>',
  mail: '<rect x="3.5" y="5.5" width="17" height="13" rx="2"/><path d="M4 7l8 6 8-6"/>',
  drive: '<path d="M8.5 4.5h7l5 8.5-3.5 6h-10l-3.5-6z"/><path d="M8.5 4.5l5 8.5h7M3.5 13h7"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5M12 7.8h.01" stroke-width="1.9"/>',
  warning: '<path d="M12 4l9 15.5H3z"/><path d="M12 10v4.5M12 17h.01" stroke-width="1.9"/>',
};

/** The glyph each kind of result shows when it has no icon of its own. */
export const KIND_GLYPHS: Record<string, string> = {
  app: "app",
  file: "file",
  folder: "folder",
  command: "command",
  ask_ai: "sparkle",
  answer: "calculator",
  clip: "clipboard",
  snippet: "snippet",
  emoji: "emoji",
  link: "link",
  process: "process",
  window: "window",
  conversation: "chat",
  script: "script",
};

/** How a result's `icon` value should be drawn. */
export type IconSpec =
  | { type: "image"; key: string }
  | { type: "glyph"; markup: string; accent: boolean }
  | { type: "text"; text: string }
  | { type: "color"; color: string };

const HEX_COLOR = /^#[0-9a-fA-F]{3,8}$/;

/** Decodes an `icon` value (see `ResultItem::icon` in Rust). */
export function iconSpec(kind: string, icon: string | null): IconSpec {
  if (icon?.startsWith("glyph:")) {
    const name = icon.slice("glyph:".length);
    const markup = GLYPHS[name];
    if (markup) return { type: "glyph", markup, accent: name === "sparkle" || name === "chatgpt" };
  } else if (icon?.startsWith("text:")) {
    const text = icon.slice("text:".length);
    if (text) return { type: "text", text };
  } else if (icon?.startsWith("color:")) {
    const color = icon.slice("color:".length);
    if (HEX_COLOR.test(color)) return { type: "color", color };
  } else if (icon) {
    return { type: "image", key: icon };
  }
  return fallback(kind);
}

/** The kind's own glyph. */
export function fallback(kind: string): IconSpec {
  const name = KIND_GLYPHS[kind] ?? "app";
  return { type: "glyph", markup: GLYPHS[name] ?? "", accent: kind === "ask_ai" };
}
