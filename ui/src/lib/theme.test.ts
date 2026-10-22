import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { AppearanceSettings } from "./bindings";
import {
  applyTheme,
  contrast,
  customAccent,
  DARK_MIN_CONTRAST,
  LIGHT_MIN_CONTRAST,
  normalizeTint,
  parseHex,
  PRESETS,
  REFERENCE,
  themeFor,
  toHex,
  type Rgb,
} from "./theme";

const hex = (text: string): Rgb => {
  const rgb = parseHex(text);
  if (!rgb) throw new Error(`not a color: ${text}`);
  return rgb;
};

const appearance = (over: Partial<AppearanceSettings> = {}): AppearanceSettings => ({
  tint: "blue",
  glass: "normal",
  density: "comfortable",
  ...over,
});

// Read as text: Vitest leaves CSS imports empty.
const tokens = readFileSync(new URL("../styles/tokens.css", import.meta.url), "utf8");
/** tokens.css split at the dark theme. */
const [lightCss = "", darkCss = ""] = tokens.split("@media (prefers-color-scheme: dark)");

/** `--name: #rrggbb` values in a block of CSS. */
function colors(css: string): Map<string, string> {
  return new Map([...css.matchAll(/(--[\w-]+):\s*(#[0-9a-f]{6})\b/gi)].map((m) => [m[1] ?? "", m[2] ?? ""]));
}

describe("parseHex / normalizeTint", () => {
  it("reads #rrggbb and #rgb in any case", () => {
    expect(parseHex("#12ab34")).toEqual({ r: 0x12, g: 0xab, b: 0x34 });
    expect(parseHex(" 12AB34 ")).toEqual({ r: 0x12, g: 0xab, b: 0x34 });
    expect(parseHex("#abc")).toEqual({ r: 0xaa, g: 0xbb, b: 0xcc });
    for (const bad of ["", "#", "#12ab3", "#12ab345", "red", "#ggg"]) expect(parseHex(bad)).toBeNull();
    expect(toHex(hex("#ABC"))).toBe("#aabbcc");
  });

  it("keeps presets and colors, and falls back to blue", () => {
    expect(normalizeTint("Purple")).toBe("purple");
    expect(normalizeTint("#ABC")).toBe("#aabbcc");
    expect(normalizeTint("ff8800")).toBe("#ff8800");
    expect(normalizeTint("")).toBe("blue");
    expect(normalizeTint("magenta")).toBe("blue");
  });
});

describe("contrast", () => {
  it("runs from 1 to 21", () => {
    expect(contrast(hex("#000"), hex("#fff"))).toBeCloseTo(21);
    expect(contrast(hex("#fff"), hex("#000"))).toBeCloseTo(21);
    expect(contrast(hex("#777"), hex("#777"))).toBe(1);
  });
});

describe("customAccent", () => {
  it("gives the default blue the look tokens.css has", () => {
    const light = colors(lightCss);
    const dark = colors(darkCss);
    const a = customAccent(hex(light.get("--tint-blue") ?? ""));
    expect(toHex(a.light)).toBe(light.get("--tint-blue"));
    expect(toHex(a.onLight)).toBe(light.get("--on-accent"));
    // Within a step of rounding of the hand-picked dark blue.
    const darkBlue = hex(dark.get("--tint-blue") ?? "");
    expect(Math.abs(a.dark.r - darkBlue.r) + Math.abs(a.dark.g - darkBlue.g) + Math.abs(a.dark.b - darkBlue.b)).toBeLessThan(4);
    expect(toHex(a.onDark)).toBe(dark.get("--on-accent"));
  });

  it("darkens a pale color for the light theme and keeps its hue", () => {
    const yellow = hex("#ffd400");
    const a = customAccent(yellow);
    expect(contrast(a.light, hex("#fff"))).toBeGreaterThanOrEqual(LIGHT_MIN_CONTRAST);
    expect(a.light.r).toBeGreaterThan(a.light.b);
    expect(a.light.g).toBeGreaterThan(a.light.b);
    // Already light enough for the dark theme: only the usual lift.
    expect(a.dark.r).toBe(255);
    expect(contrast(a.dark, hex(REFERENCE.darkSurface))).toBeGreaterThanOrEqual(DARK_MIN_CONTRAST);
  });

  it("lifts a dark color for the dark theme", () => {
    const navy = hex("#001133");
    const a = customAccent(navy);
    expect(a.light).toEqual(navy);
    expect(contrast(a.dark, hex(REFERENCE.darkSurface))).toBeGreaterThanOrEqual(DARK_MIN_CONTRAST);
  });

  it("keeps text readable for any color", () => {
    const steps = [0x00, 0x33, 0x66, 0x99, 0xcc, 0xff];
    for (const r of steps)
      for (const g of steps)
        for (const b of steps) {
          const a = customAccent({ r, g, b });
          const at = toHex({ r, g, b });
          expect(contrast(a.light, hex(REFERENCE.lightSurface)), at).toBeGreaterThanOrEqual(LIGHT_MIN_CONTRAST);
          expect(contrast(a.onLight, a.light), at).toBeGreaterThanOrEqual(LIGHT_MIN_CONTRAST);
          expect(contrast(a.dark, hex(REFERENCE.darkSurface)), at).toBeGreaterThanOrEqual(DARK_MIN_CONTRAST);
          expect(contrast(a.onDark, a.dark), at).toBeGreaterThanOrEqual(DARK_MIN_CONTRAST);
        }
  });
});

describe("tokens.css", () => {
  const light = colors(lightCss);
  const dark = colors(darkCss);

  it("has every preset in both themes, readable", () => {
    for (const name of PRESETS) {
      const l = light.get(`--tint-${name}`);
      const d = dark.get(`--tint-${name}`);
      expect(l, `light ${name}`).toBeDefined();
      expect(d, `dark ${name}`).toBeDefined();
      expect(contrast(hex(l ?? ""), hex("#fff")), `light ${name}`).toBeGreaterThanOrEqual(LIGHT_MIN_CONTRAST);
      expect(contrast(hex(d ?? ""), hex(REFERENCE.darkSurface)), `dark ${name}`).toBeGreaterThanOrEqual(DARK_MIN_CONTRAST);
      expect(contrast(hex(d ?? ""), hex(REFERENCE.onDark)), `dark ${name}`).toBeGreaterThanOrEqual(DARK_MIN_CONTRAST);
    }
  });

  it("has a rule for each preset but the default", () => {
    for (const name of PRESETS.filter((p) => p !== "blue")) {
      expect(tokens).toContain(`:root[data-tint="${name}"] {\n  --accent: var(--tint-${name});`);
    }
    expect(tokens).toContain(':root[data-tint="custom"]');
  });

  it("matches the reference colors used for custom tints", () => {
    expect(light.get("--on-accent")).toBe(REFERENCE.onLight);
    expect(dark.get("--on-accent")).toBe(REFERENCE.onDark);
    expect(dark.get("--surface")).toBe(REFERENCE.darkSurface);
  });

  it("has a rule for each glass strength but normal, and for compact", () => {
    for (const strength of ["subtle", "strong"]) {
      expect(lightCss).toContain(`[data-glass-strength="${strength}"]`);
      expect(darkCss).toContain(`[data-glass-strength="${strength}"]`);
    }
    expect(tokens).toContain(':root[data-density="compact"]');
  });
});

describe("themeFor / applyTheme", () => {
  it("names a preset and sets nothing inline", () => {
    expect(themeFor(appearance({ tint: "teal", glass: "strong", density: "compact" }))).toEqual({
      tint: "teal",
      glass: "strong",
      density: "compact",
      vars: {},
    });
    expect(themeFor(appearance({ tint: "" })).tint).toBe("blue");
  });

  it("computes a custom tint", () => {
    const theme = themeFor(appearance({ tint: "#FF8800" }));
    expect(theme.tint).toBe("custom");
    expect(Object.keys(theme.vars).sort()).toEqual([
      "--on-tint-custom",
      "--on-tint-custom-dark",
      "--tint-custom",
      "--tint-custom-dark",
    ]);
  });

  it("sets and clears the attributes and variables", () => {
    // Just what applyTheme touches on <html>.
    const vars = new Map<string, string>();
    const root = {
      dataset: {} as Record<string, string>,
      style: {
        setProperty: (name: string, value: string) => void vars.set(name, value),
        removeProperty: (name: string) => (vars.delete(name), ""),
      },
    };
    const html = root as unknown as HTMLElement;
    applyTheme(html, themeFor(appearance({ tint: "#123456", glass: "subtle", density: "compact" })));
    expect(root.dataset).toEqual({ tint: "custom", glassStrength: "subtle", density: "compact" });
    expect(vars.get("--tint-custom")).toMatch(/^#[0-9a-f]{6}$/);
    expect(vars.size).toBe(4);

    applyTheme(html, themeFor(appearance()));
    expect(root.dataset).toEqual({ tint: "blue", glassStrength: "normal", density: "comfortable" });
    expect(vars.size).toBe(0);
  });
});
