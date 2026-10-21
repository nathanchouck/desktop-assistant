import katex from "katex";
import type { MarkedExtension, Tokens } from "marked";

/**
 * LaTeX math in AI answers, drawn with KaTeX: `$…$` and `\(…\)` inline,
 * `$$…$$` and `\[…\]` as a block. Claude tends to write dollars, ChatGPT
 * the backslash forms.
 *
 * Dollars are also money, so `$…$` only counts as math when it can't be a
 * price: no space just inside either dollar, and no digit right after the
 * closing one ("costs $5 and $10" stays text).
 */

interface MathToken extends Tokens.Generic {
  type: "inlineMath" | "blockMath";
  raw: string;
  tex: string;
}

const INLINE_DOLLAR = /^\$(?!\s)((?:\\\$|[^$\n])+?)(?<![\s\\])\$(?!\d)/;
const INLINE_PAREN = /^\\\(([\s\S]+?)\\\)/;
const BLOCK_DOLLAR = /^\$\$([\s\S]+?)\$\$[^\S\n]*(?:\n|$)/;
const BLOCK_BRACKET = /^\\\[([\s\S]+?)\\\][^\S\n]*(?:\n|$)/;

/** KaTeX's HTML for `tex`; a typo shows the source in red, not an error. */
export function renderTex(tex: string, display: boolean): string {
  return katex.renderToString(tex.trim(), {
    displayMode: display,
    throwOnError: false,
    output: "html",
    // No \href, \url or \includegraphics: answers never link or load
    // anything through math.
    trust: false,
    strict: "ignore",
    maxExpand: 1000,
    maxSize: 50,
  });
}

export const mathExtension: MarkedExtension = {
  extensions: [
    {
      name: "blockMath",
      level: "block",
      start(src: string) {
        const i = src.search(/\$\$|\\\[/);
        return i < 0 ? undefined : i;
      },
      tokenizer(src: string): MathToken | undefined {
        const match = BLOCK_DOLLAR.exec(src) ?? BLOCK_BRACKET.exec(src);
        if (!match?.[1]?.trim()) return undefined;
        return { type: "blockMath", raw: match[0], tex: match[1] };
      },
      renderer(token) {
        return `<div class="math">${renderTex((token as MathToken).tex, true)}</div>\n`;
      },
    },
    {
      name: "inlineMath",
      level: "inline",
      start(src: string) {
        const i = src.search(/\$|\\\(/);
        return i < 0 ? undefined : i;
      },
      tokenizer(src: string): MathToken | undefined {
        // "$$x$$" inside a paragraph is display math written inline.
        const display = /^\$\$([^$]+?)\$\$/.exec(src);
        if (display?.[1]?.trim()) return { type: "inlineMath", raw: display[0], tex: display[1], display: true };
        const match = INLINE_DOLLAR.exec(src) ?? INLINE_PAREN.exec(src);
        if (!match?.[1]?.trim()) return undefined;
        return { type: "inlineMath", raw: match[0], tex: match[1] };
      },
      renderer(token) {
        return renderTex((token as MathToken).tex, Boolean(token.display));
      },
    },
  ],
};
