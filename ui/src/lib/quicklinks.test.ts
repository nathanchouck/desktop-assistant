import { describe, expect, it } from "vitest";
import { cleanLink, linkProblem } from "./quicklinks";

const yt = { keyword: "yt", name: "YouTube", url: "https://www.youtube.com/results?search_query={query}" };

describe("linkProblem", () => {
  it("accepts a good link", () => {
    expect(linkProblem({ keyword: "gh", name: "GitHub", url: "https://github.com/search?q={query}" }, [yt])).toBeNull();
    expect(linkProblem({ keyword: "mail", name: "Gmail", url: "HTTPS://mail.google.com" }, [])).toBeNull();
  });

  it("needs a keyword without spaces, colons or a clash", () => {
    expect(linkProblem({ ...yt, keyword: " " }, [])).toMatch(/keyword/);
    expect(linkProblem({ ...yt, keyword: "y t" }, [])).toMatch(/spaces/);
    expect(linkProblem({ ...yt, keyword: "y:t" }, [])).toMatch(/colons/);
    expect(linkProblem({ ...yt, keyword: "YT" }, [yt])).toMatch(/already a keyword/);
    expect(linkProblem({ ...yt, keyword: "c" }, [])).toMatch(/Claude/);
  });

  it("doesn't clash with itself when editing", () => {
    expect(linkProblem({ ...yt, name: "YT" }, [yt], 0)).toBeNull();
  });

  it("needs a name and a web address", () => {
    expect(linkProblem({ ...yt, name: "" }, [])).toMatch(/name/);
    expect(linkProblem({ ...yt, url: "file:///C:/Windows" }, [])).toMatch(/https/);
    expect(linkProblem({ ...yt, url: "javascript:alert(1)" }, [])).toMatch(/https/);
    expect(linkProblem({ ...yt, url: "https://" }, [])).toMatch(/https/);
    expect(linkProblem({ ...yt, url: "https://a b" }, [])).toMatch(/https/);
  });
});

describe("cleanLink", () => {
  it("trims every field", () => {
    expect(cleanLink({ keyword: " yt ", name: " YouTube ", url: " https://x.com " })).toEqual({
      keyword: "yt",
      name: "YouTube",
      url: "https://x.com",
    });
  });
});
