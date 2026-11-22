// UI text in every language. Each language is a folder (`en`, `ru`, ...) with one file per
// area of the app and an index that puts them together; English is the reference, and tsc
// checks that every language has every key. A new folder is a new language: it is found
// here and offered in settings by itself.
//
// Components read text with `useT()`, plain functions called while rendering with `tr()`.
// Never keep a translated string at module scope: the language can change while the app runs.
import { createContext, useContext } from "react";
import type { Dict } from "./en";

export type { Dict };
/** A language code: the name of its folder. */
export type Lang = string;
/** What the user picked: a language, or the system's. */
export type LangSetting = Lang | "system";

type Module = { default: Dict; meta: { name: string; locale: string } };
const found = import.meta.glob<Module>("./*/index.ts", { eager: true });
const dicts: Record<string, Module> = Object.fromEntries(Object.entries(found).map(([path, m]) => [path.split("/")[1]!, m]));

/** Every language there is, English first, the others by their own names. */
export const languages = (): { code: Lang; name: string }[] =>
  Object.entries(dicts)
    .map(([code, m]) => ({ code, name: m.meta.name }))
    .sort((a, b) => (a.code === "en" ? -1 : b.code === "en" ? 1 : a.name.localeCompare(b.name, "en")));

/** The first of the system's languages Detour has (by its whole code, then by its language). */
export function systemLang(): Lang {
  const langs = typeof navigator === "undefined" ? [] : navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const l of langs) {
    const code = l?.toLowerCase() ?? "";
    if (dicts[code]) return code;
    const base = code.split("-")[0]!;
    if (dicts[base]) return base;
  }
  return "en";
}

export function resolveLang(setting: LangSetting | undefined): Lang {
  return setting && setting !== "system" && dicts[setting] ? setting : systemLang();
}

/** The language saved in settings, read before anything renders. */
function savedLang(): Lang {
  try {
    return resolveLang(JSON.parse(localStorage.getItem("detour.settings") ?? "{}").language);
  } catch {
    return systemLang();
  }
}

let current: Lang = savedLang();
if (typeof document !== "undefined") document.documentElement.lang = current;

export function setLang(lang: Lang) {
  current = dicts[lang] ? lang : "en";
  document.documentElement.lang = current;
}

export const lang = () => current;
/** Locale for `toLocaleString` and `Intl`. */
export const locale = () => dicts[current]!.meta.locale;
/** Text in the current language, for code outside components. */
export const tr = (): Dict => dicts[current]!.default;

export const LangContext = createContext<Lang>(current);
/** Text in the current language; the component renders again when it changes. */
export const useT = (): Dict => (dicts[useContext(LangContext)] ?? dicts.en!).default;
