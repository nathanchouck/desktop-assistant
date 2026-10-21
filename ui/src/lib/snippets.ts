import type { Snippet } from "./bindings";

/** Placeholders a snippet's text can use (see `features/snippets/expand.rs`). */
export const PLACEHOLDERS = ["{clipboard}", "{date}", "{time}", "{datetime}"];

/** Whether `keyword` expands as you type with this trigger (mirrors `typing::expansions`). */
export function expandsAsYouType(keyword: string, trigger: string): boolean {
  const k = keyword.trim();
  const t = trigger.trim();
  return k.startsWith(t) && [...k].length > [...t].length;
}

/**
 * Problems with each snippet's keyword, by index: the same keyword twice,
 * and (while expanding as you type) keywords that can't expand: missing
 * the trigger, or cut off by a shorter keyword that expands first.
 */
export function snippetWarnings(items: Snippet[], trigger: string, expanding: boolean): string[][] {
  const keywords = items.map((s) => s.keyword.trim());
  const lower = keywords.map((k) => k.toLowerCase());
  const live = keywords.map((k) => expanding && expandsAsYouType(k, trigger));
  return keywords.map((keyword, i) => {
    const warnings: string[] = [];
    if (!keyword) return warnings;
    const mine = keyword.toLowerCase();
    const twin = lower.findIndex((k, j) => j !== i && k === mine);
    if (twin >= 0) {
      warnings.push(`Same keyword as “${items[twin]?.name || keywords[twin]}”.`);
    }
    if (expanding && !live[i]) {
      warnings.push(`Doesn't start with ${trigger.trim()}, so it only works from search.`);
    }
    if (live[i]) {
      // Expansion fires as soon as a keyword is typed, so one that ends
      // partway through this keyword always wins.
      const chars = [...mine];
      const blocker = lower.findIndex((other, j) => {
        if (j === i || !live[j] || other === mine) return false;
        return chars.some((_, end) => end < chars.length - 1 && chars.slice(0, end + 1).join("").endsWith(other));
      });
      if (blocker >= 0) {
        warnings.push(`Never expands: “${keywords[blocker]}” expands first while you type it.`);
      }
    }
    return warnings;
  });
}
