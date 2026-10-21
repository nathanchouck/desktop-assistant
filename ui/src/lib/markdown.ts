import DOMPurify from "dompurify";
import { Marked } from "marked";
import { mathExtension } from "./math";

const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ESCAPES[c] ?? c);
}

const marked = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    // Code blocks get a Copy button; the chat pane handles its clicks.
    code({ text }) {
      return `<pre><button type="button" class="copy">Copy</button><code>${escapeHtml(text)}</code></pre>`;
    },
    // Images become links: the page never loads remote content (the CSP
    // blocks it too), so an answer can't leak data through an image URL.
    image({ href, text }) {
      return `<a href="${escapeHtml(href)}">${escapeHtml(text || href)}</a>`;
    },
  },
});
marked.use(mathExtension);

/** An answer's Markdown (with LaTeX math) as safe HTML: rendered, then
 *  sanitized. Forms are dropped too, since submitting one would navigate
 *  the launcher away. */
export function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false });
  return DOMPurify.sanitize(html, {
    // SVG for KaTeX's drawn parts (square roots, long arrows).
    USE_PROFILES: { html: true, svg: true },
    FORBID_TAGS: ["form", "input", "textarea", "select", "img", "style", "image", "use", "foreignObject"],
  });
}
