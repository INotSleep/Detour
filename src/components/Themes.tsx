import { Fragment, useState } from "react";
import { ArrowLeft, Check, ChevronRight, Copy, Download, Pencil, Plus, Share2, Trash2 } from "lucide-react";
import { useT } from "../i18n";
import { copyText, uid, useStore } from "../lib/store";
import { SAME_AS_APP, SYSTEM, appColors, builtinThemes, isHex, readableOn, normHex, parseTheme, terminalColors, themeCode, type Theme } from "../lib/themes";
import { Button, Field, IconButton, Modal } from "./ui";

type Kind = "app" | "terminal";

/** Detour's own two looks, for previews (the stylesheet holds them for the app). */
const DETOUR_LOOK: Record<string, Look> = {
  "detour-dark": { bg: "#111215", sidebar: "#0c0d0f", raised: "#1d1f24", text: "#ececef", muted: "#6c707a", accent: "#7a8cff", line: "rgba(255, 255, 255, 0.07)" },
  "detour-light": { bg: "#fbfbfc", sidebar: "#f2f2f4", raised: "#ffffff", text: "#17181c", muted: "#8a8d96", accent: "#4d5ee8", line: "rgba(10, 12, 20, 0.08)" },
};
interface Look {
  bg: string;
  sidebar: string;
  raised: string;
  text: string;
  muted: string;
  accent: string;
  line: string;
}
const lookOf = (theme: Theme): Look => {
  const v = appColors(theme);
  if (!v) return DETOUR_LOOK[theme.id]!;
  return { bg: v["--bg"]!, sidebar: v["--sidebar"]!, raised: v["--raised"]!, text: v["--text"]!, muted: v["--muted"]!, accent: v["--accent"]!, line: v["--line-strong"]! };
};

const settingKey = (kind: Kind) => (kind === "app" ? "theme" : "terminalTheme");

/* ---------- the rows in Settings ---------- */

/** A Theme row: the theme in use, and the picker behind it. */
export function ThemeSetting({ kind }: { kind: Kind }) {
  const t = useT().settings;
  const { settings, themes } = useStore();
  const [picking, setPicking] = useState(false);
  const value = settings[settingKey(kind)];
  const shown = kind === "app" ? themes.app : themes.terminal;
  const label = value === SYSTEM ? t.picker.system : value === SAME_AS_APP ? t.picker.sameAsApp : shown.name;
  return (
    <div className="setting">
      <span>{kind === "app" ? t.theme : t.terminalTheme}</span>
      <button className="theme-current" onClick={() => setPicking(true)}>
        <Swatch theme={shown} />
        <span>{label}</span>
        <ChevronRight size={14} className="muted" />
      </button>
      {picking && <ThemePicker kind={kind} onClose={() => setPicking(false)} />}
    </div>
  );
}

function Swatch({ theme }: { theme: Theme }) {
  const c = terminalColors(theme);
  return (
    <span className="theme-swatch" style={{ background: c.background }}>
      {[c.red, c.green, c.yellow, c.blue, c.magenta, c.cyan].map((col, i) => <i key={i} style={{ background: col }} />)}
    </span>
  );
}

/* ---------- picking, making, sharing and importing ---------- */

type View = { kind: "grid" } | { kind: "edit"; draft: Theme } | { kind: "import" };

function ThemePicker({ kind, onClose }: { kind: Kind; onClose: () => void }) {
  const all = useT();
  const t = all.settings.picker;
  const { settings, setSettings, themes, toast } = useStore();
  const [view, setView] = useState<View>({ kind: "grid" });
  const key = settingKey(kind);
  const value = settings[key];
  const custom = settings.customThemes;
  const choose = (id: string) => setSettings({ [key]: id });

  const share = async (theme: Theme) =>
    toast((await copyText(themeCode(theme))) ? { text: t.copied(theme.name), tone: "success" } : { text: all.common.copyFailed, tone: "error" });

  const remove = (theme: Theme) => {
    const before = settings;
    const fallbackId = kind === "app" ? "detour-dark" : SAME_AS_APP;
    setSettings({
      customThemes: custom.filter((x) => x.id !== theme.id),
      theme: settings.theme === theme.id ? "detour-dark" : settings.theme,
      terminalTheme: settings.terminalTheme === theme.id ? SAME_AS_APP : settings.terminalTheme,
      ...(value === theme.id ? { [key]: fallbackId } : {}),
    });
    toast({ text: t.deleted(theme.name), action: { label: t.undo, run: () => setSettings({ customThemes: before.customThemes, theme: before.theme, terminalTheme: before.terminalTheme }) } });
  };

  const save = (draft: Theme) => {
    const exists = custom.some((x) => x.id === draft.id);
    setSettings({ customThemes: exists ? custom.map((x) => (x.id === draft.id ? draft : x)) : [...custom, draft], [key]: draft.id });
    setView({ kind: "grid" });
  };

  const current = kind === "app" ? themes.app : themes.terminal;
  const copyOf = (theme: Theme): Theme => ({ id: uid("theme-"), name: t.copyName(theme.name), custom: true, c: [...theme.c] });

  if (view.kind === "edit") return <ThemeEditor draft={view.draft} onSave={save} onBack={() => setView({ kind: "grid" })} onClose={onClose} />;
  if (view.kind === "import")
    return (
      <ImportTheme
        onBack={() => setView({ kind: "grid" })}
        onClose={onClose}
        onImported={(theme) => {
          setSettings({ customThemes: [...custom, theme], [key]: theme.id });
          toast({ text: t.added(theme.name), tone: "success" });
          setView({ kind: "grid" });
        }}
      />
    );

  const special = kind === "app" ? { id: SYSTEM, name: t.system } : { id: SAME_AS_APP, name: t.sameAsApp };
  const card = (theme: Theme, actions: { icon: typeof Copy; label: string; run: () => void }[]) => {
    const on = value === theme.id;
    return (
      <div key={theme.id} className={`theme-card ${on ? "on" : ""}`}>
        <button type="button" className="theme-card-pick" aria-pressed={on} onClick={() => choose(theme.id)}>
          <Preview kind={kind} theme={theme} />
        </button>
        <span className="theme-card-name">
          <span className="ellipsis">{theme.name}</span>
          <span className="theme-card-actions">
            {actions.map((a) => <IconButton key={a.label} type="button" icon={a.icon} label={a.label} onClick={a.run} />)}
          </span>
          {on && <Check size={14} className="theme-card-check" />}
        </span>
      </div>
    );
  };

  return (
    <Modal
      title={kind === "app" ? t.appTitle : t.terminalTitle}
      width={780}
      onClose={onClose}
      onSubmit={onClose}
      footer={
        <>
          <Button type="button" icon={Plus} onClick={() => setView({ kind: "edit", draft: copyOf(current) })}>{t.newTheme}</Button>
          <Button type="button" icon={Download} onClick={() => setView({ kind: "import" })}>{t.import}</Button>
          <span className="grow" />
          <Button type="submit" variant="primary">{t.done}</Button>
        </>
      }
    >
      <div className="theme-scroll">
        <div className="theme-grid">
          <div className={`theme-card ${value === special.id ? "on" : ""}`}>
            <button type="button" className="theme-card-pick" aria-pressed={value === special.id} onClick={() => choose(special.id)}>
              {kind === "app" ? (
                <span className="theme-split">
                  <Preview kind="app" theme={builtinThemes[0]!} />
                  <Preview kind="app" theme={builtinThemes[1]!} />
                </span>
              ) : (
                <Preview kind="terminal" theme={themes.app} />
              )}
            </button>
            <span className="theme-card-name">
              <span className="ellipsis">{special.name}</span>
              {value === special.id && <Check size={14} className="theme-card-check" />}
            </span>
          </div>
          {builtinThemes.map((th) => card(th, [
            { icon: Share2, label: t.share, run: () => void share(th) },
            { icon: Pencil, label: t.editCopy, run: () => setView({ kind: "edit", draft: copyOf(th) }) },
          ]))}
        </div>
        {custom.length > 0 && (
          <>
            <div className="theme-section">{t.yours}</div>
            <div className="theme-grid">
              {custom.map((th) => card(th, [
                { icon: Share2, label: t.share, run: () => void share(th) },
                { icon: Pencil, label: t.edit, run: () => setView({ kind: "edit", draft: { ...th, c: [...th.c] } }) },
                { icon: Trash2, label: all.common.delete, run: () => remove(th) },
              ]))}
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}

/* ---------- making a theme ---------- */

function ThemeEditor({ draft: initial, onSave, onBack, onClose }: { draft: Theme; onSave: (t: Theme) => void; onBack: () => void; onClose: () => void }) {
  const t = useT().settings.picker;
  const [draft, setDraft] = useState(initial);
  const set = (i: number, v: string) => setDraft((d) => ({ ...d, c: d.c.map((x, j) => (j === i ? v : x)) }));
  const base = [t.background, t.text, t.cursor, t.selection];
  const ready = !!draft.name.trim();
  return (
    <Modal
      title={t.editTitle}
      width={780}
      onClose={onClose}
      onSubmit={() => ready && onSave({ ...draft, name: draft.name.trim(), custom: true })}
      footer={
        <>
          <Button type="button" variant="ghost" icon={ArrowLeft} onClick={onBack}>{t.back}</Button>
          <span className="grow" />
          <Button type="submit" variant="primary" disabled={!ready}>{t.save}</Button>
        </>
      }
    >
      <div className="theme-editor">
        <div className="theme-editor-previews">
          <Preview kind="app" theme={draft} />
          <Preview kind="terminal" theme={draft} />
        </div>
        <Field label={t.name}>
          <input className="input" value={draft.name} onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))} maxLength={60} />
        </Field>
        <div className="theme-base">
          {base.map((label, i) => (
            <ColorField key={label} label={label} value={draft.c[i]!} onChange={(v) => set(i, v)} />
          ))}
        </div>
        <div className="theme-ansi">
          {[0, 1].map((col) => (
            <div key={col} className="theme-ansi-col">
              <span />
              <span className="theme-ansi-head">{t.normal}</span>
              <span className="theme-ansi-head">{t.bright}</span>
              {t.roles.slice(col * 4, col * 4 + 4).map((role, k) => {
                const i = col * 4 + k;
                return (
                  <Fragment key={role}>
                    <span className="theme-ansi-label">{role}</span>
                    <ColorDot value={draft.c[4 + i]!} onChange={(v) => set(4 + i, v)} />
                    <ColorDot value={draft.c[12 + i]!} onChange={(v) => set(12 + i, v)} />
                  </Fragment>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    </Modal>
  );
}

/** A colour with a picker and its hex code, for pasting. */
function ColorField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const [text, setText] = useState(value);
  const rgbOnly = value.slice(0, 7);
  return (
    <Field label={label}>
      <div className="color-field">
        <label className="color-dot" style={{ background: value }}>
          <input type="color" value={rgbOnly} onChange={(e) => { onChange(e.target.value); setText(e.target.value); }} />
        </label>
        <input
          className="input mono"
          value={text}
          spellCheck={false}
          onChange={(e) => {
            setText(e.target.value);
            if (isHex(e.target.value)) onChange(normHex(e.target.value));
          }}
          onBlur={() => setText(value)}
        />
      </div>
    </Field>
  );
}

/** A colour to click and pick, its code written on it. */
function ColorDot({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <label className="color-dot big" style={{ background: value, color: readableOn(value) }}>
      <span className="mono">{value.slice(1, 7)}</span>
      <input type="color" value={value.slice(0, 7)} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

/* ---------- importing ---------- */

function ImportTheme({ onImported, onBack, onClose }: { onImported: (t: Theme) => void; onBack: () => void; onClose: () => void }) {
  const t = useT().settings.picker;
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const submit = () => {
    const theme = parseTheme(text, uid("theme-"));
    if (!theme) return setError(t.notATheme);
    onImported(theme);
  };
  return (
    <Modal
      title={t.importTitle}
      width={560}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" icon={ArrowLeft} onClick={onBack}>{t.back}</Button>
          <span className="grow" />
          <Button type="submit" variant="primary" disabled={!text.trim()}>{t.import}</Button>
        </>
      }
    >
      <Field label={t.importLabel} error={error}>
        <textarea
          className="input textarea mono"
          rows={6}
          value={text}
          autoFocus
          spellCheck={false}
          placeholder="detour-theme:1:…"
          onChange={(e) => {
            setText(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            // Enter adds a line; Ctrl+Enter imports.
            if (e.key === "Enter" && !(e.ctrlKey || e.metaKey)) e.stopPropagation();
          }}
        />
      </Field>
    </Modal>
  );
}

/* ---------- previews ---------- */

/** The theme as a small window (the app) or a small terminal. */
function Preview({ kind, theme }: { kind: Kind; theme: Theme }) {
  return kind === "app" ? <AppPreview theme={theme} /> : <TermPreview theme={theme} />;
}

function AppPreview({ theme }: { theme: Theme }) {
  const l = lookOf(theme);
  const term = terminalColors(theme);
  return (
    <span className="app-preview" style={{ background: l.bg, color: l.text, borderColor: l.line }}>
      <span className="app-preview-side" style={{ background: l.sidebar, borderColor: l.line }}>
        <i style={{ background: l.accent }} />
        <i style={{ background: l.muted }} />
        <i style={{ background: l.muted }} />
        <i style={{ background: l.muted }} />
      </span>
      <span className="app-preview-main">
        <span className="app-preview-title" style={{ background: l.text }} />
        <span className="app-preview-row" style={{ background: l.raised, borderColor: l.line }}>
          <i style={{ background: l.muted }} />
          <b style={{ background: l.accent }} />
        </span>
        <span className="app-preview-term mono" style={{ background: term.background, color: term.foreground, borderColor: l.line }}>
          <span style={{ color: term.green }}>me@api-1</span>:<span style={{ color: term.blue }}>~</span>$ <span className="theme-cursor" style={{ background: term.cursor }} />
        </span>
      </span>
    </span>
  );
}

/** A prompt, an `ls` and the cursor, in the theme's colours. */
function TermPreview({ theme }: { theme: Theme }) {
  const c = terminalColors(theme);
  const prompt = (
    <>
      <span style={{ color: c.green }}>me@api-1</span>
      <span>:</span>
      <span style={{ color: c.blue }}>~</span>
      <span>$ </span>
    </>
  );
  return (
    <span className="theme-preview mono" style={{ background: c.background, color: c.foreground }}>
      <span>{prompt}ls</span>
      <span>
        <span style={{ color: c.blue }}>app</span>{"  "}
        <span style={{ color: c.blue }}>logs</span>{"  "}
        <span style={{ color: c.green }}>deploy.sh</span>{"  "}
        <span>notes.md</span>
      </span>
      <span>
        <span style={{ color: c.red }}>error</span>{" "}
        <span style={{ color: c.yellow }}>warn</span>{" "}
        <span style={{ color: c.magenta }}>v2.4</span>{" "}
        <span style={{ color: c.cyan }}>200 OK</span>
      </span>
      <span>
        {prompt}
        <span className="theme-cursor" style={{ background: c.cursor }} />
      </span>
    </span>
  );
}

