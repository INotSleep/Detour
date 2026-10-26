// Colour themes for the app and its terminals. A theme is one palette: the
// terminal's colours, from which the app's own colours are derived, so any
// theme (your own too) suits both. Names of known themes are theirs, not translated.
import type { ITheme } from "@xterm/xterm";

export interface Theme {
  id: string;
  name: string;
  /** Made, imported or changed here. */
  custom?: boolean;
  /** Background, foreground, cursor, selection, then the 16 ANSI colours (normal, bright): "#rrggbb", selection may add alpha. */
  c: string[];
}

const ANSI = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

const t = (id: string, name: string, base: string, normal: string, bright: string): Theme => ({ id, name, c: [...base.split(" "), ...normal.split(" "), ...bright.split(" ")] });

export const builtinThemes: Theme[] = [
  t("detour-dark", "Detour Dark", "#0b0c0e #d9dbe1 #8d9cff #7a8cff52", "#1b1d22 #ef6461 #4cc38a #e5a54b #7a8cff #c58af9 #56c7d8 #c9ccd5", "#5d616c #ff8582 #6fd8a4 #f2c178 #9eabff #d8acff #7fdcea #f2f3f6"),
  t("detour-light", "Detour Light", "#fbfbfc #17181c #4d5ee8 #4d5ee833", "#17181c #d6453f #2f9e66 #b7791f #4d5ee8 #9a4fd0 #1d8fa3 #8a8d96", "#4f525a #e5534b #38b174 #c98a2a #6573f0 #ad66de #2aa5ba #c9cbd1"),
  t("one-dark", "One Dark", "#282c34 #abb2bf #528bff #3e4451", "#282c34 #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #abb2bf", "#5c6370 #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #ffffff"),
  t("dracula", "Dracula", "#282a36 #f8f8f2 #f8f8f2 #44475a", "#21222c #ff5555 #50fa7b #f1fa8c #bd93f9 #ff79c6 #8be9fd #f8f8f2", "#6272a4 #ff6e6e #69ff94 #ffffa5 #d6acff #ff92df #a4ffff #ffffff"),
  t("nord", "Nord", "#2e3440 #d8dee9 #d8dee9 #434c5e", "#3b4252 #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #88c0d0 #e5e9f0", "#4c566a #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #8fbcbb #eceff4"),
  t("tokyo-night", "Tokyo Night", "#1a1b26 #c0caf5 #c0caf5 #33467c", "#15161e #f7768e #9ece6a #e0af68 #7aa2f7 #bb9af7 #7dcfff #a9b1d6", "#414868 #f7768e #9ece6a #e0af68 #7aa2f7 #bb9af7 #7dcfff #c0caf5"),
  t("catppuccin-mocha", "Catppuccin Mocha", "#1e1e2e #cdd6f4 #f5e0dc #45475a", "#45475a #f38ba8 #a6e3a1 #f9e2af #89b4fa #f5c2e7 #94e2d5 #bac2de", "#585b70 #f38ba8 #a6e3a1 #f9e2af #89b4fa #f5c2e7 #94e2d5 #a6adc8"),
  t("gruvbox-dark", "Gruvbox Dark", "#282828 #ebdbb2 #ebdbb2 #504945", "#282828 #cc241d #98971a #d79921 #458588 #b16286 #689d6a #a89984", "#928374 #fb4934 #b8bb26 #fabd2f #83a598 #d3869b #8ec07c #ebdbb2"),
  t("monokai", "Monokai", "#272822 #f8f8f2 #f8f8f0 #49483e", "#272822 #f92672 #a6e22e #f4bf75 #66d9ef #ae81ff #a1efe4 #f8f8f2", "#75715e #f92672 #a6e22e #f4bf75 #66d9ef #ae81ff #a1efe4 #f9f8f5"),
  t("tomorrow-night", "Tomorrow Night", "#1d1f21 #c5c8c6 #c5c8c6 #373b41", "#1d1f21 #cc6666 #b5bd68 #f0c674 #81a2be #b294bb #8abeb7 #c5c8c6", "#969896 #cc6666 #b5bd68 #f0c674 #81a2be #b294bb #8abeb7 #ffffff"),
  t("solarized-dark", "Solarized Dark", "#002b36 #839496 #93a1a1 #073642", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5", "#586e75 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3"),
  t("solarized-light", "Solarized Light", "#fdf6e3 #657b83 #586e75 #eee8d5", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5", "#002b36 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3"),
  t("one-light", "One Light", "#fafafa #383a42 #526fff #e5e5e6", "#383a42 #e45649 #50a14f #c18401 #4078f2 #a626a4 #0184bc #a0a1a7", "#696c77 #e45649 #50a14f #c18401 #4078f2 #a626a4 #0184bc #fafafa"),
  t("github-light", "GitHub Light", "#ffffff #24292f #0969da #b6d7ff", "#24292f #cf222e #116329 #4d2d00 #0969da #8250df #1b7c83 #6e7781", "#57606a #a40e26 #1a7f37 #633c01 #218bff #a475f9 #3192aa #8c959f"),
];

/** The pseudo themes a setting can name instead of a theme. */
export const SYSTEM = "system";
export const SAME_AS_APP = "app";

export const findTheme = (id: string, custom: Theme[]) => custom.find((x) => x.id === id) ?? builtinThemes.find((x) => x.id === id);

/* ---------- colour arithmetic ---------- */

const rgb = (hex: string): [number, number, number] => {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? h.split("").map((x) => x + x).join("") : h.slice(0, 6);
  const n = parseInt(full, 16) || 0;
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
};
const hex = (r: number, g: number, b: number) => `#${[r, g, b].map((v) => Math.round(Math.max(0, Math.min(255, v))).toString(16).padStart(2, "0")).join("")}`;
/** `a` moved toward `b` by `k` (0..1). */
const mix = (a: string, b: string, k: number) => {
  const [x, y] = [rgb(a), rgb(b)];
  return hex(x[0] + (y[0] - x[0]) * k, x[1] + (y[1] - x[1]) * k, x[2] + (y[2] - x[2]) * k);
};
const alpha = (c: string, a: number) => `rgba(${rgb(c).join(", ")}, ${a})`;
const luminance = (c: string) => {
  const [r, g, b] = rgb(c).map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
};

export const isLight = (theme: Theme) => luminance(theme.c[0]!) > 0.45;
/** Black or white, whichever reads better on `c`. */
export const readableOn = (c: string) => (luminance(c) > 0.35 ? "#111111" : "#ffffff");
/** WCAG contrast of two colours, 1..21. */
const contrast = (a: string, b: string) => {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};
export const isHex = (s: string) => /^#?[0-9a-f]{6}([0-9a-f]{2})?$/i.test(s.trim());
export const normHex = (s: string) => `#${s.trim().replace("#", "").toLowerCase()}`;

/* ---------- what a theme gives the terminal and the app ---------- */

export function terminalColors(theme: Theme): ITheme & { background: string; foreground: string } {
  const [bg, fg, cursor, sel, ...ansi] = theme.c as [string, string, string, string, ...string[]];
  const out: Record<string, string> = { background: bg, foreground: fg, cursor, cursorAccent: bg, selectionBackground: sel.length === 7 ? `${sel}66` : sel };
  ANSI.forEach((name, i) => {
    out[name] = ansi[i] ?? fg;
    out[`bright${name[0]!.toUpperCase()}${name.slice(1)}`] = ansi[i + 8] ?? fg;
  });
  return out as unknown as ITheme & { background: string; foreground: string };
}

/**
 * The app's colours for a theme. Detour's own two are the stylesheet's; any
 * other is derived from its palette: its background, text, blue as the accent.
 */
export function appColors(theme: Theme): Record<string, string> | null {
  if (theme.id === "detour-dark" || theme.id === "detour-light") return null;
  const [bg, fg, , , , red, green, yellow, blue] = theme.c as string[];
  const light = isLight(theme);
  // Blue is the accent; the bright one when plain blue barely shows on the background.
  const brightBlue = theme.c[16]!;
  const accent = contrast(blue!, bg!) >= 3 || contrast(brightBlue, bg!) <= contrast(blue!, bg!) ? blue! : brightBlue;
  const common = {
    "--bg": bg!,
    "--text": fg!,
    "--text-2": mix(fg!, bg!, 0.3),
    "--muted": mix(fg!, bg!, 0.5),
    "--accent": accent,
    "--accent-soft": alpha(accent, light ? 0.1 : 0.15),
    "--on-accent": luminance(accent) > 0.45 ? mix(bg!, "#000000", light ? 0.8 : 0.3) : "#ffffff",
    "--ok": green!,
    "--warn": yellow!,
    "--err": red!,
    "--err-soft": alpha(red!, light ? 0.08 : 0.12),
    "--warn-soft": alpha(yellow!, 0.1),
    "--hover": alpha(fg!, light ? 0.045 : 0.05),
    "--press": alpha(fg!, light ? 0.08 : 0.09),
    "--line": alpha(fg!, light ? 0.09 : 0.08),
    "--line-strong": alpha(fg!, light ? 0.15 : 0.15),
  };
  return light
    ? { ...common, "--sidebar": mix(bg!, fg!, 0.04), "--surface": mix(bg!, "#ffffff", 0.6), "--raised": mix(bg!, "#ffffff", 0.7), "--input": mix(bg!, "#ffffff", 0.6), "--accent-hover": mix(accent, "#000000", 0.1), "--shadow": `0 16px 40px ${alpha(fg!, 0.14)}, 0 0 0 1px var(--line-strong)` }
    : { ...common, "--sidebar": mix(bg!, "#000000", 0.22), "--surface": mix(bg!, fg!, 0.04), "--raised": mix(bg!, fg!, 0.07), "--input": mix(bg!, "#000000", 0.15), "--accent-hover": mix(accent, fg!, 0.15), "--shadow": "0 16px 40px rgba(0, 0, 0, 0.5), 0 0 0 1px var(--line-strong)" };
}

/* ---------- sharing: a short code, or a Windows Terminal colour scheme ---------- */

/** One line to paste in a chat: `detour-theme:1:<name>:<20 colours>`. */
export const themeCode = (theme: Theme) => `detour-theme:1:${encodeURIComponent(theme.name)}:${theme.c.map((x) => x.replace("#", "")).join(".")}`;

const WT_KEYS = ["black", "red", "green", "yellow", "blue", "purple", "cyan", "white"];

/** A theme from a code or a Windows Terminal scheme (JSON), as a new theme of this device; null if it is neither. */
export function parseTheme(text: string, id: string): Theme | null {
  const code = text.match(/detour-theme:1:([^:\s]*):([0-9a-f.]+)/i);
  if (code) {
    const colours = code[2]!.split(".").filter(Boolean);
    if (colours.length !== 20 || !colours.every(isHex)) return null;
    let name: string;
    try {
      name = decodeURIComponent(code[1]!);
    } catch {
      name = code[1]!;
    }
    return { id, name: name.trim().slice(0, 60) || "Theme", custom: true, c: colours.map(normHex) };
  }
  try {
    const j = JSON.parse(text.trim());
    const s = Array.isArray(j?.schemes) ? j.schemes[0] : j;
    const get = (k: string) => (typeof s?.[k] === "string" && isHex(s[k]) ? normHex(s[k]) : null);
    const base = [get("background"), get("foreground"), get("cursorColor") ?? get("foreground"), get("selectionBackground") ?? get("brightBlack")];
    const ansi = [...WT_KEYS.map((k) => get(k)), ...WT_KEYS.map((k) => get(`bright${k[0]!.toUpperCase()}${k.slice(1)}`))];
    const all = [...base, ...ansi];
    if (all.some((x) => x === null)) return null;
    return { id, name: String(s.name ?? "Theme").trim().slice(0, 60) || "Theme", custom: true, c: all as string[] };
  } catch {
    return null;
  }
}
