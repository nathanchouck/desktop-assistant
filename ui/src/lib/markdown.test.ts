// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

describe("renderMarkdown", () => {
  it("renders Markdown", () => {
    expect(renderMarkdown("**hi** [site](https://example.com)")).toBe(
      '<p><strong>hi</strong> <a href="https://example.com">site</a></p>\n',
    );
  });

  it("gives code blocks a copy button and escapes them", () => {
    const html = renderMarkdown("```\n<b>x</b>\n```");
    expect(html).toContain('<button type="button" class="copy">Copy</button>');
    expect(html).toContain("&lt;b&gt;x&lt;/b&gt;");
  });

  it("strips scripts, handlers, forms and images", () => {
    const html = renderMarkdown(
      '<img src=x onerror="alert(1)"><script>alert(2)</script><form action="https://x"><input name=a></form>',
    );
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("<script");
    expect(html).not.toContain("<form");
    expect(html).not.toContain("<input");
    expect(html).not.toContain("<img");
  });

  it("turns Markdown images into links", () => {
    expect(renderMarkdown("![chart](https://example.com/c.png?d=secret)")).toBe(
      '<p><a href="https://example.com/c.png?d=secret">chart</a></p>\n',
    );
  });
});

describe("math", () => {
  it("draws inline and block LaTeX", () => {
    const inline = renderMarkdown("Euler: $e^{i\\pi} + 1 = 0$ and \\(x^2\\)");
    expect(inline).toContain('class="katex"');
    expect(inline.match(/class="katex"/g)).toHaveLength(2);
    const block = renderMarkdown("$$\n\\int_0^1 x\\,dx = \\tfrac12\n$$");
    expect(block).toContain('class="katex-display"');
    expect(renderMarkdown("\\[\\sqrt{2}\\]")).toContain("katex-display");
  });

  it("leaves prices alone", () => {
    expect(renderMarkdown("It costs $5 and $10.")).toBe("<p>It costs $5 and $10.</p>\n");
    expect(renderMarkdown("$ 5 $")).not.toContain("katex");
  });

  it("keeps math inside code as code", () => {
    const html = renderMarkdown("`$x$` and\n```\n$$y$$\n```");
    expect(html).not.toContain("katex");
    expect(html).toContain("$x$");
  });

  it("shows bad LaTeX as text instead of failing", () => {
    expect(renderMarkdown("$\\frac{1}{$ ok")).toContain("ok");
  });

  it("never lets math link or load anything", () => {
    const html = renderMarkdown("$\\href{javascript:alert(1)}{x}$ $\\includegraphics{https://x/y.png}$");
    expect(html).not.toContain("javascript:");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<a ");
  });
});
