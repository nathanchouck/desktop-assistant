/** What a key press in the search box means. Pure, so it's easy to test. */
export type Intent =
  | { type: "move"; delta: number }
  | { type: "activate"; index?: number; admin: boolean }
  | { type: "escape" };

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

export function intentFor(e: KeyLike): Intent | null {
  switch (e.key) {
    case "ArrowDown":
      return { type: "move", delta: 1 };
    case "ArrowUp":
      return { type: "move", delta: -1 };
    case "Enter":
      // Ctrl+Shift+Enter is the Windows convention for "run as administrator".
      return { type: "activate", admin: e.ctrlKey && e.shiftKey };
    case "Escape":
      return { type: "escape" };
  }
  // Ctrl+1..9 opens that row directly.
  if (e.ctrlKey && !e.altKey && !e.metaKey && /^[1-9]$/.test(e.key)) {
    return { type: "activate", index: Number(e.key) - 1, admin: false };
  }
  return null;
}
