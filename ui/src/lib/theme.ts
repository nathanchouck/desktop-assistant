// Theme variants (Settings → Appearance) on top of tokens.css.
//
// tokens.css holds every color and size, including the accent presets in
// light and dark. This module only picks among them by setting attributes
// on <html>, and computes a custom accent's light and dark versions, since
// those can't be written down ahead of time.

import type { AppearanceSettings, Density, GlassStrength } from "./bindings";

export const PRESETS = ["blue", "purple", "pink", "red", "orange", "green", "teal", "graphite"] as const;
export type Preset = (typeof PRESETS)[number];

export const PRESET_NAMES: Record<Preset, string> = {
  blue: "Blue",
  purple: "Purple",
  pink: "Pink",
  red: "Red",
  orange: "Orange",
  green: "Green",
  teal: "Teal",
  graphite: "Graphite",
};

export type Rgb = { r: number; g: number; b: number };

/** The tokens.css colors a custom accent is checked against (theme.test.ts
 * keeps these in step with the file). */
export const REFERENCE = {
  /** The lightest the light theme gets behind text: glass over white. */
  lightSurface: "#ffffff",
  /** The dark theme's opaque surface. */
  darkSurface: "#1c1e23",
  /** Text on the accent: light theme, then dark theme. */
  onLight: "#ffffff",
  onDark: "#0d1020",
};

/** Light theme: white text on the accent, and the accent as link text on a
 * near-white surface, read at least this well (the default blue: 3.7). */
export const LIGHT_MIN_CONTRAST = 3.5;
/** Dark theme: the accent as text on the dark surface. */
export const DARK_MIN_CONTRAST = 4.5;
/** How much lighter the dark theme's accent starts (blue: #4f7cff → #7c9cff). */
const DARK_LIFT = 0.25;

/** What goes on <html>. */
export interface Theme {
  /** `data-tint`: a preset, or "custom" with `vars`. */
  tint: Preset | "custom";
  glass: GlassStrength;
  density: Density;
  /** Custom properties set inline (only for a custom tint). */
  vars: Record<string, string>;
}

/** The custom properties a custom tint sets. */
export const CUSTOM_VARS = ["--tint-custom", "--on-tint-custom", "--tint-custom-dark", "--on-tint-custom-dark"] as const;

/** "#rrggbb" or "#rgb" (any case, # optional), else null. */
export function parseHex(text: string): Rgb | null {
  const hex = text.trim().replace(/^#/, "");
  const full = /^[0-9a-f]{3}$/i.test(hex)
    ? [...hex].map((c) => c + c).join("")
    : /^[0-9a-f]{6}$/i.test(hex)
      ? hex
      : null;
  if (full === null) return null;
  const n = Number.parseInt(full, 16);
  return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}

export function toHex({ r, g, b }: Rgb): string {
  return "#" + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
}

/** A tint setting as stored: a preset name or a custom color ("#12ab34"),
 * lower case. Anything else (empty, a typo) is blue. */
export function normalizeTint(tint: string): Preset | `#${string}` {
  const name = tint.trim().toLowerCase();
  if ((PRESETS as readonly string[]).includes(name)) return name as Preset;
  const rgb = parseHex(name);
  return rgb ? (toHex(rgb) as `#${string}`) : "blue";
}

/** WCAG relative luminance. */
export function luminance({ r, g, b }: Rgb): number {
  const lin = (v: number) => {
    const s = v / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio, 1 to 21. */
export function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** `a` moved toward `b` by `t` (0..1), in sRGB. */
export function mix(a: Rgb, b: Rgb, t: number): Rgb {
  return {
    r: Math.round(a.r + (b.r - a.r) * t),
    g: Math.round(a.g + (b.g - a.g) * t),
    b: Math.round(a.b + (b.b - a.b) * t),
  };
}

/** The first step from `start` toward `toward` that `ok` accepts; the
 * color keeps its hue and only gets darker or lighter as needed. */
function adjust(start: Rgb, toward: Rgb, ok: (c: Rgb) => boolean): Rgb {
  for (let t = 0; t < 1; t += 0.05) {
    const c = mix(start, toward, t);
    if (ok(c)) return c;
  }
  return toward;
}

/** Text on `background`: the theme's usual text color when it reads well
 * enough, else whichever of the two reads better. */
function textOn(background: Rgb, preferred: Rgb, other: Rgb, min: number): Rgb {
  if (contrast(background, preferred) >= min) return preferred;
  return contrast(background, preferred) >= contrast(background, other) ? preferred : other;
}

/** A custom accent's light and dark theme colors and the text on each.
 * Light: darkened until it reads on white (and white reads on it). Dark:
 * lifted a little, as the presets are, then until it reads on the dark
 * surface. */
export function customAccent(base: Rgb): {
  light: Rgb;
  onLight: Rgb;
  dark: Rgb;
  onDark: Rgb;
} {
  const white = rgb(REFERENCE.lightSurface);
  const black = { r: 0, g: 0, b: 0 };
  const darkSurface = rgb(REFERENCE.darkSurface);
  const onLight = rgb(REFERENCE.onLight);
  const onDark = rgb(REFERENCE.onDark);
  const light = adjust(base, black, (c) => contrast(c, white) >= LIGHT_MIN_CONTRAST);
  const lifted = mix(base, white, DARK_LIFT);
  const dark = adjust(lifted, white, (c) => contrast(c, darkSurface) >= DARK_MIN_CONTRAST);
  return {
    light,
    onLight: textOn(light, onLight, onDark, LIGHT_MIN_CONTRAST),
    dark,
    onDark: textOn(dark, onDark, onLight, DARK_MIN_CONTRAST),
  };
}

/** The theme for the appearance settings. */
export function themeFor(appearance: AppearanceSettings): Theme {
  const tint = normalizeTint(appearance.tint);
  const base = { glass: appearance.glass, density: appearance.density };
  if (!tint.startsWith("#")) return { ...base, tint: tint as Preset, vars: {} };
  const accent = customAccent(rgb(tint));
  return {
    ...base,
    tint: "custom",
    vars: {
      "--tint-custom": toHex(accent.light),
      "--on-tint-custom": toHex(accent.onLight),
      "--tint-custom-dark": toHex(accent.dark),
      "--on-tint-custom-dark": toHex(accent.onDark),
    },
  };
}

/** Puts `theme` on `root` (<html>); tokens.css does the rest. */
export function applyTheme(root: HTMLElement, theme: Theme): void {
  root.dataset.tint = theme.tint;
  root.dataset.glassStrength = theme.glass;
  root.dataset.density = theme.density;
  for (const name of CUSTOM_VARS) {
    const value = theme.vars[name];
    if (value) root.style.setProperty(name, value);
    else root.style.removeProperty(name);
  }
}

function rgb(hex: string): Rgb {
  return parseHex(hex) ?? { r: 0, g: 0, b: 0 };
}
