import { useState } from "react";
import { Code, Download, Ellipsis, Lock, Pencil, Play, Plus, RefreshCw, Trash2 } from "lucide-react";
import { locale, useT } from "../i18n";
import { api, errorText, isCoreError } from "../lib/api";
import { uid, useStore } from "../lib/store";
import { atLeast, writable } from "../lib/teams";
import type { Snippet } from "../lib/types";
import { Button, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented, Switch } from "./ui";
import { SyncSettings } from "./SyncSettings";
import { useEditors } from "./Editors";
import { ThemeSetting } from "./Themes";
import { isNoReleases } from "../lib/updates";

/* ---------- snippets ---------- */

export function SnippetsView() {
  const { ws, vault, update, toast, connect } = useStore();
  const all = useT();
  const t = all.settings.snippets;
  const canCreate = writable(vault);
  const [editing, setEditing] = useState<Snippet | "new" | null>(null);
  const snippets = ws.snippets.filter((s) => s.vaultId === vault.id);
  const hosts = ws.hosts.filter((h) => h.vaultId === vault.id);

  const remove = (s: Snippet) => {
    update((w) => ({ ...w, snippets: w.snippets.filter((x) => x.id !== s.id) }));
    toast({ text: t.removed(s.name), action: { label: t.restore, run: () => update((w) => ({ ...w, snippets: [...w.snippets, s] })) } });
  };

  return (
    <div className="page">
      <PageHeader title={t.title} count={snippets.length || undefined}>
        {canCreate && <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>{t.create}</Button>}
      </PageHeader>

      {snippets.length === 0 && <Empty icon={Code} title={t.empty} action={canCreate ? <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>{t.create}</Button> : undefined} />}

      <div className="rows">
        {snippets.map((s) => {
          const canEdit = atLeast(s.access, "edit");
          return (
          <div className="item-row" key={s.id} onDoubleClick={() => canEdit && setEditing(s)}>
            <span className="item-icon"><Code size={15} /></span>
            <div className="item-main">
              <span className="item-title">{s.name}</span>
              <span className="item-sub mono">{s.command}</span>
            </div>
            <div className="row-actions visible">
              {hosts.length > 0 && (
                <Menu
                  header={t.runOn}
                  trigger={(toggle, open) => <Button size="sm" icon={Play} className={open ? "pressed" : ""} onClick={toggle}>{t.run}</Button>}
                  items={hosts.map((h) => ({ label: h.label, hint: h.online === false ? t.unavailable : undefined, onSelect: () => connect(h.id, s.command) }))}
                />
              )}
              {canEdit && (
                <Menu
                  trigger={(toggle, open) => <IconButton icon={Ellipsis} label={t.more} className={open ? "pressed" : ""} onClick={toggle} />}
                  items={[
                    { label: all.common.edit, icon: Pencil, onSelect: () => setEditing(s) },
                    { label: all.common.delete, icon: Trash2, danger: true, onSelect: () => remove(s) },
                  ]}
                />
              )}
            </div>
          </div>
          );
        })}
      </div>

      {editing && <SnippetDialog snippet={editing === "new" ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function SnippetDialog({ snippet, onClose }: { snippet: Snippet | null; onClose: () => void }) {
  const { vault, update, toast } = useStore();
  const all = useT();
  const t = all.settings.snippets;
  const [name, setName] = useState(snippet?.name ?? "");
  const [command, setCommand] = useState(snippet?.command ?? "");
  const valid = name.trim() && command.trim();

  const submit = () => {
    if (!valid) return;
    if (snippet) {
      update((w) => ({ ...w, snippets: w.snippets.map((s) => (s.id === snippet.id ? { ...s, name: name.trim(), command: command.trim() } : s)) }));
      toast({ text: t.saved, tone: "success" });
    } else {
      update((w) => ({ ...w, snippets: [...w.snippets, { id: uid("s"), vaultId: vault.id, name: name.trim(), command: command.trim() }] }));
      toast({ text: t.created(name.trim()), tone: "success" });
    }
    onClose();
  };

  return (
    <Modal
      title={snippet ? t.dialogTitle : t.newTitle}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!valid}>{snippet ? all.common.save : all.common.create}</Button>
        </>
      }
    >
      <Field label={t.name}>
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder={t.namePlaceholder} />
      </Field>
      <Field label={t.command}>
        <textarea
          className="input mono textarea"
          rows={3}
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && (e.ctrlKey || e.metaKey) && submit()}
          placeholder="sudo systemctl restart nginx"
          spellCheck={false}
        />
      </Field>
    </Modal>
  );
}

/* ---------- settings ---------- */

export function SettingsView() {
  const { settings, setSettings, coreInfo, lock, update_: update, checkUpdate, installUpdate } = useStore();
  const t = useT().settings;
  const [changingMaster, setChangingMaster] = useState(false);
  return (
    <div className="page narrow">
      <PageHeader title={t.title} />

      <section className="settings-group">
        <h3 className="section-title">{t.appearance}</h3>
        <ThemeSetting kind="app" />
        <div className="setting">
          <span>{t.language}</span>
          <Segmented value={settings.language} onChange={(language) => setSettings({ language })} options={[{ value: "system", label: t.languages.system }, { value: "en", label: t.languages.en }, { value: "ru", label: t.languages.ru }]} />
        </div>
      </section>

      <section className="settings-group">
        <h3 className="section-title">{t.terminal}</h3>
        <ThemeSetting kind="terminal" />
        <div className="setting">
          <span>{t.fontSize}</span>
          <select className="input input-sm" value={settings.fontSize} onChange={(e) => setSettings({ fontSize: Number(e.target.value) })}>
            {[11, 12, 13, 14, 15, 16, 18].map((n) => <option key={n} value={n}>{n}</option>)}
          </select>
        </div>
        <div className="setting">
          <span>{t.scrollback}</span>
          <select className="input input-sm" value={settings.scrollback} onChange={(e) => setSettings({ scrollback: Number(e.target.value) })}>
            {[1000, 5000, 10000, 50000].map((n) => <option key={n} value={n}>{t.lines(n, n.toLocaleString(locale()))}</option>)}
          </select>
        </div>
        <div className="setting">
          <span>{t.clipboard}</span>
          <Segmented value={settings.clipboard} onChange={(clipboard) => setSettings({ clipboard })} options={[{ value: "ssh", label: t.clipboardSsh }, { value: "termius", label: t.clipboardTermius }]} />
        </div>
        <div className="setting">
          <span>{t.highlight}</span>
          <Switch label={t.highlight} checked={settings.highlight} onChange={(highlight) => setSettings({ highlight })} />
        </div>
      </section>

      <section className="settings-group">
        <h3 className="section-title">{t.files}</h3>
        <div className="setting">
          <span>{t.editWith}</span>
          <EditorSetting />
        </div>
      </section>

      <SyncSettings />

      {api.real && (
      <section className="settings-group">
        <h3 className="section-title">{t.security}</h3>
        <div className="setting">
          <span>{t.masterPassword}</span>
          <Button size="sm" onClick={() => setChangingMaster(true)}>{t.change}</Button>
        </div>
        <div className="setting">
          <span>{t.autoLock}</span>
          <select className="input input-sm" value={settings.autoLockMinutes} onChange={(e) => setSettings({ autoLockMinutes: Number(e.target.value) })}>
            <option value={5}>{t.minutes(5)}</option>
            <option value={15}>{t.minutes(15)}</option>
            <option value={60}>{t.hour}</option>
            <option value={0}>{t.never}</option>
          </select>
        </div>
        <div className="setting">
          <span>{t.lockNow}</span>
          <Button size="sm" icon={Lock} onClick={() => void lock()}>{t.lock}</Button>
        </div>
      </section>
      )}

      <section className="settings-group">
        <h3 className="section-title">{t.about}</h3>
        <div className="setting">
          <span>{t.version}</span>
          <span className="muted mono">{coreInfo ? `${coreInfo.coreVersion} · ${coreInfo.os}` : "0.2.1"}</span>
        </div>
        <div className="setting">
          <span>{t.license}</span>
          <span className="muted mono selectable">AGPL-3.0 · github.com/INotSleep/Detour</span>
        </div>
        {api.real && (
          <>
            <div className="setting">
              <span>{t.updates}</span>
              <div className="update-row">
                {update.kind === "latest" && <span className="muted small">{t.latest}</span>}
                {update.kind === "error" && <span className={`small ${isNoReleases(update.text) ? "muted" : "err-text"}`}>{update.text}</span>}
                {update.kind === "installing" && <span className="muted small">{update.share === null ? t.downloading : t.downloadingShare(Math.round(update.share * 100))}</span>}
                {update.kind === "available" || update.kind === "installing" ? (
                  <Button size="sm" variant="primary" icon={Download} loading={update.kind === "installing"} onClick={() => void installUpdate()}>
                    {t.install(update.info.version)}
                  </Button>
                ) : (
                  <Button size="sm" icon={RefreshCw} loading={update.kind === "checking"} onClick={() => void checkUpdate()}>{t.check}</Button>
                )}
              </div>
            </div>
            <div className="setting">
              <span>{t.checkAtStart}</span>
              <Switch label={t.checkAtStart} checked={settings.autoUpdate} onChange={(autoUpdate) => setSettings({ autoUpdate })} />
            </div>
          </>
        )}
      </section>
      {changingMaster && <MasterPasswordDialog onClose={() => setChangingMaster(false)} />}
    </div>
  );
}

function MasterPasswordDialog({ onClose }: { onClose: () => void }) {
  const { toast } = useStore();
  const all = useT();
  const t = all.settings.master;
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [error, setError] = useState<{ field: "current" | "next" | "repeat"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (busy) return;
    if (next.length < 8) return setError({ field: "next", text: t.minLength(8) });
    if (next !== repeat) return setError({ field: "repeat", text: t.mismatch });
    setBusy(true);
    try {
      await api.vaultChangeMasterPassword(current, next);
      toast({ text: t.changed, tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      setError(isCoreError(e) && e.kind === "wrongMasterPassword" ? { field: "current", text: errorText(e) } : { field: "next", text: errorText(e) });
    }
  };

  const err = (f: "current" | "next" | "repeat") => (error?.field === f ? error.text : null);
  const edit = (set: (v: string) => void) => (v: string) => {
    set(v);
    setError(null);
  };

  return (
    <Modal
      title={t.title}
      width={400}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!current || !next}>{all.settings.change}</Button>
        </>
      }
    >
      <Field label={t.current} error={err("current")}>
        <PasswordInput value={current} onChange={edit(setCurrent)} invalid={!!err("current")} autoFocus />
      </Field>
      <Field label={t.next} error={err("next")}>
        <PasswordInput value={next} onChange={edit(setNext)} invalid={!!err("next")} />
      </Field>
      <Field label={t.repeat} error={err("repeat")}>
        <PasswordInput value={repeat} onChange={edit(setRepeat)} invalid={!!err("repeat")} />
      </Field>
    </Modal>
  );
}

/** What "Edit" opens files in: the system's app for each file, or one editor. */
function EditorSetting() {
  const t = useT().settings;
  const { settings, setSettings } = useStore();
  const installed = useEditors();
  const list = settings.editor && !installed.some((e) => e.path === settings.editor!.path) ? [settings.editor, ...installed] : installed;
  return (
    <select className="input input-sm" value={settings.editor?.path ?? ""} onChange={(e) => setSettings({ editor: list.find((x) => x.path === e.target.value) ?? null })}>
      <option value="">{t.defaultApp}</option>
      {list.map((e) => <option key={e.path} value={e.path}>{e.name}</option>)}
    </select>
  );
}
