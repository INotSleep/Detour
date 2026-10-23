import { useState } from "react";
import { Code, Ellipsis, Lock, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { api, errorText, isCoreError } from "../lib/api";
import { uid, useStore } from "../lib/store";
import { writable } from "../lib/teams";
import type { Snippet } from "../lib/types";
import { Button, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented } from "./ui";

/* ---------- snippets ---------- */

export function SnippetsView() {
  const { ws, vault, update, toast, connect } = useStore();
  const canEdit = writable(vault);
  const [editing, setEditing] = useState<Snippet | "new" | null>(null);
  const snippets = ws.snippets.filter((s) => s.vaultId === vault.id);
  const hosts = ws.hosts.filter((h) => h.vaultId === vault.id);

  const remove = (s: Snippet) => {
    update((w) => ({ ...w, snippets: w.snippets.filter((x) => x.id !== s.id) }));
    toast({ text: `«${s.name}» удалён`, action: { label: "Вернуть", run: () => update((w) => ({ ...w, snippets: [...w.snippets, s] })) } });
  };

  return (
    <div className="page">
      <PageHeader title="Сниппеты" count={snippets.length || undefined}>
        {canEdit && <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>Создать сниппет</Button>}
      </PageHeader>

      {snippets.length === 0 && <Empty icon={Code} title="Нет сниппетов" action={canEdit ? <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>Создать сниппет</Button> : undefined} />}

      <div className="rows">
        {snippets.map((s) => (
          <div className="item-row" key={s.id} onDoubleClick={() => canEdit && setEditing(s)}>
            <span className="item-icon"><Code size={15} /></span>
            <div className="item-main">
              <span className="item-title">{s.name}</span>
              <span className="item-sub mono">{s.command}</span>
            </div>
            <div className="row-actions visible">
              {hosts.length > 0 && (
                <Menu
                  header="Запустить на"
                  trigger={(toggle, open) => <Button size="sm" icon={Play} className={open ? "pressed" : ""} onClick={toggle}>Запустить</Button>}
                  items={hosts.map((h) => ({ label: h.label, hint: h.online === false ? "недоступен" : undefined, onSelect: () => connect(h.id, s.command) }))}
                />
              )}
              {canEdit && (
                <Menu
                  trigger={(toggle, open) => <IconButton icon={Ellipsis} label="Ещё" className={open ? "pressed" : ""} onClick={toggle} />}
                  items={[
                    { label: "Изменить", icon: Pencil, onSelect: () => setEditing(s) },
                    { label: "Удалить", icon: Trash2, danger: true, onSelect: () => remove(s) },
                  ]}
                />
              )}
            </div>
          </div>
        ))}
      </div>

      {editing && <SnippetDialog snippet={editing === "new" ? null : editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function SnippetDialog({ snippet, onClose }: { snippet: Snippet | null; onClose: () => void }) {
  const { vault, update, toast } = useStore();
  const [name, setName] = useState(snippet?.name ?? "");
  const [command, setCommand] = useState(snippet?.command ?? "");
  const valid = name.trim() && command.trim();

  const submit = () => {
    if (!valid) return;
    if (snippet) {
      update((w) => ({ ...w, snippets: w.snippets.map((s) => (s.id === snippet.id ? { ...s, name: name.trim(), command: command.trim() } : s)) }));
      toast({ text: "Сохранено", tone: "success" });
    } else {
      update((w) => ({ ...w, snippets: [...w.snippets, { id: uid("s"), vaultId: vault.id, name: name.trim(), command: command.trim() }] }));
      toast({ text: `«${name.trim()}» создан`, tone: "success" });
    }
    onClose();
  };

  return (
    <Modal
      title={snippet ? "Сниппет" : "Новый сниппет"}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!valid}>{snippet ? "Сохранить" : "Создать"}</Button>
        </>
      }
    >
      <Field label="Название">
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Перезапуск nginx" />
      </Field>
      <Field label="Команда">
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
  const { settings, setSettings, coreInfo, lock } = useStore();
  const [changingMaster, setChangingMaster] = useState(false);
  return (
    <div className="page narrow">
      <PageHeader title="Настройки" />

      <section className="settings-group">
        <h3 className="section-title">Внешний вид</h3>
        <div className="setting">
          <span>Тема</span>
          <Segmented value={settings.theme} onChange={(theme) => setSettings({ theme })} options={[{ value: "system", label: "Как в системе" }, { value: "dark", label: "Тёмная" }, { value: "light", label: "Светлая" }]} />
        </div>
      </section>

      <section className="settings-group">
        <h3 className="section-title">Терминал</h3>
        <div className="setting">
          <span>Размер шрифта</span>
          <select className="input input-sm" value={settings.fontSize} onChange={(e) => setSettings({ fontSize: Number(e.target.value) })}>
            {[11, 12, 13, 14, 15, 16, 18].map((n) => <option key={n} value={n}>{n}</option>)}
          </select>
        </div>
        <div className="setting">
          <span>История прокрутки</span>
          <select className="input input-sm" value={settings.scrollback} onChange={(e) => setSettings({ scrollback: Number(e.target.value) })}>
            {[1000, 5000, 10000, 50000].map((n) => <option key={n} value={n}>{n.toLocaleString("ru-RU")} строк</option>)}
          </select>
        </div>
      </section>

      {api.real && (
      <section className="settings-group">
        <h3 className="section-title">Безопасность</h3>
        <div className="setting">
          <span>Мастер-пароль</span>
          <Button size="sm" onClick={() => setChangingMaster(true)}>Сменить</Button>
        </div>
        <div className="setting">
          <span>Блокировать при бездействии</span>
          <select className="input input-sm" value={settings.autoLockMinutes} onChange={(e) => setSettings({ autoLockMinutes: Number(e.target.value) })}>
            <option value={5}>5 минут</option>
            <option value={15}>15 минут</option>
            <option value={60}>1 час</option>
            <option value={0}>Никогда</option>
          </select>
        </div>
        <div className="setting">
          <span>Заблокировать сейчас</span>
          <Button size="sm" icon={Lock} onClick={() => void lock()}>Заблокировать</Button>
        </div>
      </section>
      )}

      <section className="settings-group">
        <h3 className="section-title">О программе</h3>
        <div className="setting">
          <span>Версия</span>
          <span className="muted mono">{coreInfo ? `${coreInfo.coreVersion} · ${coreInfo.os}` : "0.0.1"}</span>
        </div>
      </section>
      {changingMaster && <MasterPasswordDialog onClose={() => setChangingMaster(false)} />}
    </div>
  );
}

function MasterPasswordDialog({ onClose }: { onClose: () => void }) {
  const { toast } = useStore();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [error, setError] = useState<{ field: "current" | "next" | "repeat"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (busy) return;
    if (next.length < 8) return setError({ field: "next", text: "Минимум 8 символов" });
    if (next !== repeat) return setError({ field: "repeat", text: "Пароли не совпадают" });
    setBusy(true);
    try {
      await api.vaultChangeMasterPassword(current, next);
      toast({ text: "Мастер-пароль изменён", tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      setError(isCoreError(e) && e.kind === "wrongMasterPassword" ? { field: "current", text: "Неверный мастер-пароль" } : { field: "next", text: errorText(e) });
    }
  };

  const err = (f: "current" | "next" | "repeat") => (error?.field === f ? error.text : null);
  const edit = (set: (v: string) => void) => (v: string) => {
    set(v);
    setError(null);
  };

  return (
    <Modal
      title="Смена мастер-пароля"
      width={400}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!current || !next}>Сменить</Button>
        </>
      }
    >
      <Field label="Текущий" error={err("current")}>
        <PasswordInput value={current} onChange={edit(setCurrent)} invalid={!!err("current")} autoFocus />
      </Field>
      <Field label="Новый" error={err("next")}>
        <PasswordInput value={next} onChange={edit(setNext)} invalid={!!err("next")} />
      </Field>
      <Field label="Ещё раз" error={err("repeat")}>
        <PasswordInput value={repeat} onChange={edit(setRepeat)} invalid={!!err("repeat")} />
      </Field>
    </Modal>
  );
}
