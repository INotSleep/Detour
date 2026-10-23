// UI text in every language. Each namespace module exports `en` and `ru` of the same
// shape; texts that take values are functions. Components read text with `useT()`,
// plain functions called while rendering with `tr()`. Never keep a translated
// string at module scope: the language can change while the app runs.
import { createContext, useContext } from "react";
import * as app from "./app";
import * as common from "./common";
import * as demo from "./demo";
import * as files from "./files";
import * as hosts from "./hosts";
import * as keys from "./keys";
import * as settings from "./settings";
import * as sync from "./sync";
import * as teams from "./teams";
import * as terminal from "./terminal";
import * as tunnels from "./tunnels";

export type Lang = "en" | "ru";
/** What the user picked: a language, or the system's. */
export type LangSetting = Lang | "system";

const en = {
  common: common.en,
  app: app.en,
  settings: settings.en,
  sync: sync.en,
  teams: teams.en,
  files: files.en,
  terminal: terminal.en,
  hosts: hosts.en,
  keys: keys.en,
  tunnels: tunnels.en,
  demo: demo.en,
};
export type Dict = typeof en;
const ru: Dict = {
  common: common.ru,
  app: app.ru,
  settings: settings.ru,
  sync: sync.ru,
  teams: teams.ru,
  files: files.ru,
  terminal: terminal.ru,
  hosts: hosts.ru,
  keys: keys.ru,
  tunnels: tunnels.ru,
  demo: demo.ru,
};
const dicts: Record<Lang, Dict> = { en, ru };

export function systemLang(): Lang {
  const langs = typeof navigator === "undefined" ? [] : navigator.languages?.length ? navigator.languages : [navigator.language];
  return langs.some((l) => l?.toLowerCase().startsWith("ru")) ? "ru" : "en";
}

export function resolveLang(setting: LangSetting | undefined): Lang {
  return setting === "en" || setting === "ru" ? setting : systemLang();
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
  current = lang;
  document.documentElement.lang = lang;
}

export const lang = () => current;
/** Locale for `toLocaleString` and `Intl`. */
export const locale = () => (current === "ru" ? "ru-RU" : "en-US");
/** Text in the current language, for code outside components. */
export const tr = (): Dict => dicts[current];

export const LangContext = createContext<Lang>(current);
/** Text in the current language; the component renders again when it changes. */
export const useT = (): Dict => dicts[useContext(LangContext)];

export { enForm, ruForm } from "./plural";
