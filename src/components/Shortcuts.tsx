import { useEffect, useMemo, useState } from "react";
import { RotateCcw, Search, X } from "lucide-react";
import { useT } from "../i18n";
import { useStore } from "../lib/store";
import { ACTIONS, ACTION_IDS, AREAS, comboLabel, comboOf, comboOfEvent, eatsShellKey, recording, takenBy, type ActionId, type Overrides } from "../lib/shortcuts";
import { Button, IconButton, Modal } from "./ui";

/** Settings: the row that opens the list of shortcuts. */
export function ShortcutsSetting() {
  const t = useT().settings.keyboard;
  const { settings } = useStore();
  const [open, setOpen] = useState(false);
  const changed = Object.keys(settings.shortcuts ?? {}).length;
  return (
    <section className="settings-group">
      <h3 className="section-title">{t.title}</h3>
      <div className="setting">
        <span>{t.shortcuts}</span>
        <div className="update-row">
          {changed > 0 && <span className="muted small">{t.changed(changed)}</span>}
          <Button size="sm" onClick={() => setOpen(true)}>{t.edit}</Button>
        </div>
      </div>
      {open && <ShortcutsDialog onClose={() => setOpen(false)} />}
    </section>
  );
}

/** Areas where a key without Ctrl or Alt would be typed as text instead. */
const needsModifier = (id: ActionId) => ["general", "tabs", "terminal"].includes(ACTIONS[id].area);

function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  const all = useT();
  const t = all.settings.keyboard;
  const { settings, setSettings, toast } = useStore();
  const o: Overrides = settings.shortcuts ?? {};
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<ActionId | null>(null);
  const [error, setError] = useState<{ id: ActionId; text: string } | null>(null);

  const save = (next: Overrides) => setSettings({ shortcuts: next });
  const assign = (id: ActionId, combo: string) => {
    const next = { ...o };
    if (combo === ACTIONS[id].keys) delete next[id];
    else next[id] = combo;
    save(next);
  };

  // While a row waits for keys, they go to it and nowhere else.
  useEffect(() => {
    if (!editing) return;
    recording.on = true;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
        setEditing(null);
        setError(null);
        return;
      }
      const combo = comboOfEvent(e);
      if (!combo) return;
      const fKey = /^F\d+$/.test(combo.split("+").pop()!);
      if (needsModifier(editing) && !fKey && !/^(Mod|Alt)\+/.test(combo)) return setError({ id: editing, text: t.needsModifier });
      const other = takenBy(editing, combo, o);
      if (other) return setError({ id: editing, text: t.taken(t.actions[other]) });
      assign(editing, combo);
      setEditing(null);
      setError(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      recording.on = false;
    };
  }, [editing, o]); // eslint-disable-line react-hooks/exhaustive-deps

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    return AREAS.map((area) => ({
      area,
      ids: ACTION_IDS.filter((id) => ACTIONS[id].area === area && (!q || t.actions[id].toLowerCase().includes(q) || comboLabel(comboOf(id, o)).toLowerCase().includes(q))),
    })).filter((g) => g.ids.length);
  }, [query, o, t]);

  const resetAll = () => {
    const before = o;
    save({});
    toast({ text: t.resetDone, action: { label: t.undo, run: () => save(before) } });
  };

  return (
    <Modal
      title={t.dialogTitle}
      width={560}
      onClose={() => (editing ? (setEditing(null), setError(null)) : onClose())}
      footer={
        <>
          {Object.keys(o).length > 0 && <Button type="button" variant="ghost" icon={RotateCcw} onClick={resetAll}>{t.resetAll}</Button>}
          <span className="grow" />
          <Button type="button" variant="primary" onClick={onClose}>{all.common.done}</Button>
        </>
      }
    >
      <label className="search shortcuts-search">
        <Search size={14} />
        <input placeholder={t.search} value={query} onChange={(e) => setQuery(e.target.value)} />
        {query && <IconButton icon={X} label={t.clearSearch} size={13} onClick={() => setQuery("")} />}
      </label>
      <div className="shortcuts">
        {groups.length === 0 && <div className="muted small shortcuts-empty">{t.nothingFound}</div>}
        {groups.map((g) => (
          <div key={g.area} className="shortcuts-group">
            <h4 className="section-title">{t.areas[g.area]}</h4>
            {g.ids.map((id) => {
              const combo = comboOf(id, o);
              const changed = id in o;
              const err = error?.id === id ? error.text : null;
              const warn = !err && changed && eatsShellKey(id, combo) ? t.shell : null;
              return (
                <div key={id} className="shortcut-row">
                  <div className="shortcut-main">
                    <span className="grow">{t.actions[id]}</span>
                    {changed && <IconButton icon={RotateCcw} label={t.reset} size={13} onClick={() => assign(id, ACTIONS[id].keys)} />}
                    {combo && editing !== id && <IconButton icon={X} label={t.clear} size={13} onClick={() => assign(id, "")} />}
                    <button
                      type="button"
                      className={`shortcut-keys ${editing === id ? "recording" : ""} ${combo ? "" : "none"}`}
                      onClick={() => {
                        setError(null);
                        setEditing(editing === id ? null : id);
                      }}
                    >
                      {editing === id ? t.press : combo ? comboLabel(combo) : t.none}
                    </button>
                  </div>
                  {(err || warn) && <div className={`small shortcut-note ${err ? "err-text" : "muted"}`}>{err ?? warn}</div>}
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </Modal>
  );
}
