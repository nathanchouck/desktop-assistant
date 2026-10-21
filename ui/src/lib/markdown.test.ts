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
