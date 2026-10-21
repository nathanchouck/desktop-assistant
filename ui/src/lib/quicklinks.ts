import type { Quicklink } from "./bindings";

/** Prefixes other parts of the launcher already answer to. */
const RESERVED: Record<string, string> = {
  c: "asks Claude",
  g: "asks ChatGPT",
  f: "searches files",
  d: "searches folders",
};

/** The link as it should be saved: no stray spaces. */
export function cleanLink(link: Quicklink): Quicklink {
  return { keyword: link.keyword.trim(), name: link.name.trim(), url: link.url.trim() };
}

/**
 * What's wrong with `link`, or null if it can be saved. `index` is its own
 * place in `all` when editing (so it doesn't clash with itself).
 */
export function linkProblem(link: Quicklink, all: Quicklink[], index = -1): string | null {
  const { keyword, name, url } = cleanLink(link);
  if (!keyword) return "Give it a keyword, like yt.";
  if (/[\s:]/.test(keyword)) return "A keyword can't have spaces or colons.";
  const reserved = RESERVED[keyword.toLowerCase()];
  if (reserved) return `“${keyword}” plus a space already ${reserved}.`;
  const taken = all.some((other, i) => i !== index && other.keyword.trim().toLowerCase() === keyword.toLowerCase());
  if (taken) return `“${keyword}” is already a keyword.`;
  if (!name) return "Give it a name, like YouTube.";
  if (!/^https?:\/\/\S+$/i.test(url)) return "The address must start with https:// (or http://).";
  return null;
}
