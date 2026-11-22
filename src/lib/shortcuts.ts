// Every keyboard shortcut of the app in one place: what each action is, where it works and
// its keys by default. The keys someone chose instead are kept in settings (`shortcuts`);
// handlers ask `hit(event, id)` and menus show `keysOf(id)`, so a change applies everywhere.
//
// A combo is written as "Mod+Shift+D": Mod is Ctrl (Cmd on a Mac), then Alt and Shift, then
// the key. Letters and digits are read by their place on the keyboard, so they work in any layout.

/** Where an action works. General and tab actions work anywhere, the others where their area has focus. */
export type Area = "general" | "tabs" | "terminal" | "files" | "hosts";

export const ACTIONS = {
  palette: { area: "general", keys: "Mod+K" },
  settings: { area: "general", keys: "Mod+," },
  lock: { area: "general", keys: "Mod+Shift+L" },
  nextTab: { area: "tabs", keys: "Mod+Tab" },
  prevTab: { area: "tabs", keys: "Mod+Shift+Tab" },
  closeTab: { area: "tabs", keys: "Mod+Shift+W" },
  moveTabLeft: { area: "tabs", keys: "Mod+Shift+PageUp" },
  moveTabRight: { area: "tabs", keys: "Mod+Shift+PageDown" },
  copy: { area: "terminal", keys: "Mod+Shift+C" },
  paste: { area: "terminal", keys: "Mod+Shift+V" },
  find: { area: "terminal", keys: "Mod+Shift+F" },
  splitRight: { area: "terminal", keys: "Mod+Shift+D" },
  splitDown: { area: "terminal", keys: "Mod+Shift+E" },
  focusLeft: { area: "terminal", keys: "Alt+Left" },
  focusRight: { area: "terminal", keys: "Alt+Right" },
  focusUp: { area: "terminal", keys: "Alt+Up" },
  focusDown: { area: "terminal", keys: "Alt+Down" },
  selectAll: { area: "files", keys: "Mod+A" },
  editPath: { area: "files", keys: "Mod+L" },
  refresh: { area: "files", keys: "F5" },
  rename: { area: "files", keys: "F2" },
  removeFile: { area: "files", keys: "Delete" },
  parentDir: { area: "files", keys: "Backspace" },
  removeHost: { area: "hosts", keys: "Delete" },
} as const satisfies Record<string, { area: Area; keys: string }>;

export type ActionId = keyof typeof ACTIONS;
export const ACTION_IDS = Object.keys(ACTIONS) as ActionId[];
export const AREAS: Area[] = ["general", "tabs", "terminal", "files", "hosts"];

/** Keys chosen instead of the defaults; "" turns an action off. */
export type Overrides = Partial<Record<ActionId, string>>;

let overrides: Overrides = {};
/** Set by the app whenever settings change. */
export function setOverrides(o: Overrides | undefined) {
  overrides = o ?? {};
}

/** The keys of an action now; "" when it has none. */
export const comboOf = (id: ActionId, o: Overrides = overrides): string => o[id] ?? ACTIONS[id].keys;

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

const CODE_KEYS: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  Space: "Space",
};
const NAMED: Record<string, string> = { ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Esc: "Escape", Del: "Delete" };
const MODIFIERS = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph", "OS", "CapsLock"]);

/** The key of an event as a combo uses it, or null for a modifier alone. */
function keyName(e: Pick<KeyboardEvent, "code" | "key">): string | null {
  if (MODIFIERS.has(e.key)) return null;
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
  if (/^Numpad\d$/.test(e.code)) return e.code.slice(6);
  if (CODE_KEYS[e.code]) return CODE_KEYS[e.code]!;
  const k = NAMED[e.key] ?? e.key;
  return k.length === 1 ? k.toUpperCase() : k;
}

/** The combo an event is, as "Mod+Shift+D"; null while only modifiers are held. */
export function comboOfEvent(e: Pick<KeyboardEvent, "code" | "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">): string | null {
  const key = keyName(e);
  if (!key) return null;
  return [e.ctrlKey || e.metaKey ? "Mod" : "", e.altKey ? "Alt" : "", e.shiftKey ? "Shift" : "", key].filter(Boolean).join("+");
}

/** Whether an event is the shortcut of an action. */
export function hit(e: Pick<KeyboardEvent, "code" | "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">, id: ActionId): boolean {
  const c = comboOf(id);
  return !!c && comboOfEvent(e) === c;
}

const MAC_SIGNS: Record<string, string> = { Mod: "⌘", Alt: "⌥", Shift: "⇧" };
const SHOWN: Record<string, string> = { Left: "←", Right: "→", Up: "↑", Down: "↓", PageUp: "PgUp", PageDown: "PgDn", Delete: "Del", Escape: "Esc", Backspace: isMac ? "⌫" : "Backspace" };

/** How a combo is shown: "Ctrl Shift D", on a Mac "⌘⇧D". */
export function comboLabel(combo: string): string {
  if (!combo) return "";
  const parts = combo.split("+").map((p, i, all) => (i < all.length - 1 ? p : SHOWN[p] ?? p));
  // A "+" key splits into two empty parts.
  if (combo.endsWith("++")) parts.splice(-2, 2, "+");
  return isMac ? parts.map((p) => MAC_SIGNS[p] ?? p).join("") : parts.map((p) => (p === "Mod" ? "Ctrl" : p)).join(" ");
}

/** How the keys of an action are shown now; "" when it has none. */
export const keysOf = (id: ActionId) => comboLabel(comboOf(id));

/** Whether two actions can fire at the same place, so they must not share keys. */
export const sameReach = (a: ActionId, b: ActionId) => {
  const [x, y] = [ACTIONS[a].area, ACTIONS[b].area];
  const global = (z: Area) => z === "general" || z === "tabs";
  return x === y || global(x) || global(y);
};

/** The action that already has these keys where `id` works, if any. */
export function takenBy(id: ActionId, combo: string, o: Overrides): ActionId | null {
  return ACTION_IDS.find((other) => other !== id && sameReach(id, other) && comboOf(other, o) === combo) ?? null;
}

/**
 * A combo that a program on the server never gets while a terminal has focus: Ctrl with a
 * letter alone is how shells and editors are driven (Ctrl+K, Ctrl+W, Ctrl+L).
 */
export function eatsShellKey(id: ActionId, combo: string): boolean {
  const area = ACTIONS[id].area;
  if (isMac || !(area === "general" || area === "tabs" || area === "terminal")) return false;
  return /^Mod\+[A-Z\\[\]]$/.test(combo);
}

/** While a new combo is being typed in settings, nothing else reacts to keys. */
export const recording = { on: false };
