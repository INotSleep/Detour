import { useEffect, useMemo, useRef, useState } from "react";
import { hit, keysOf } from "../lib/shortcuts";
import { ChevronRight, CopyPlus, Download, Ellipsis, FolderOpen, FolderPlus, History, KeyRound, Pencil, Plus, Search, Server, Settings2, Shield, SquareTerminal, Terminal, Trash2, TriangleAlert, Upload, X } from "lucide-react";
import { api, errorText } from "../lib/api";
import { copyText, formatAgo, uid, useStore } from "../lib/store";
import { sshCommand } from "../lib/sshConfig";
import type { Host } from "../lib/types";
import { HostIcon, authLabel, groupLabel, isNoGroup } from "./common";
import { atLeast, canManage, writable } from "../lib/teams";
import { addGroup, asLoggedIn, fromGroups, layoutFrom, listGroups, listMine, mineAdd, mineMoveGroup, mineMoveHost, mineRemove, mineRename, moveGroup, moveHost, removeGroup, renameGroup, type ListedGroup } from "../lib/groups";
import { GroupSettingsDialog, groupPrefsKey } from "./GroupSettings";
import type { Layout } from "../lib/types";
import { OwnLoginSection, SavedPassword } from "./HostLogin";
import { Button, Checkbox, Empty, Field, IconButton, Menu, Modal, PageHeader, Segmented } from "./ui";
import { HostFields, applyDraft, draftFromHost, draftInvalid, emptyDraft, useHostDraft } from "./HostForm";
import { ImportDialog } from "./ImportDialog";
import { ExportDialog } from "./ExportDialog";
import { RecordHistoryDialog } from "./TeamHistory";
import { useT } from "../i18n";

export function HostsView() {
  const t = useT();
  const { ws, vault, update, toast, connect, openFiles, resolveConflicts, applyTeamView } = useStore();
  const conflicted = new Set(vault.conflicts.filter((c) => c.coll === "host").map((c) => c.record));
  const canCreate = writable(vault);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);
  /** Hosts to start the export with; null when not exporting. */
  const [exporting, setExporting] = useState<string[] | null>(null);
  const [historyOf, setHistoryOf] = useState<Host | null>(null);

  const hosts = ws.hosts.filter((h) => h.vaultId === vault.id);
  const vaultGroups = ws.groups.filter((g) => g.vaultId === vault.id);
  const groupNames = [...new Set([...vaultGroups.map((g) => g.name), ...hosts.map((h) => h.group).filter((g) => !isNoGroup(g))])];
  // In a team, the shared order and groups are for those who manage it; hosts for their editors.
  const canEditGroups = vault.kind === "personal" || canManage(vault.myRole);
  const canMoveHost = (h: Host) => vault.kind === "personal" || atLeast(h.access, "edit");
  /** A group being named: a new one (`from` null), or one renamed; `mine` for a group of mine. */
  const [naming, setNaming] = useState<{ from: string | null; mine?: string } | null>(null);
  const [settingsOf, setSettingsOf] = useState<ListedGroup | null>(null);

  // A team's hosts in its shared groups or in this person's own ones, as this device last showed.
  const viewKey = `detour.groups.${vault.id}`;
  const [mineView, setMineViewState] = useState(() => {
    try {
      return localStorage.getItem(viewKey) === "mine";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      setMineViewState(localStorage.getItem(viewKey) === "mine");
    } catch {
      /* private mode */
    }
  }, [viewKey]);
  const mine = vault.kind === "team" && mineView;
  const layout = ws.layouts[vault.id];

  /** Saves this person's own grouping of the team's hosts. */
  const saveLayout = (next: Layout) => {
    update((w) => ({ ...w, layouts: { ...w.layouts, [vault.id]: next } }));
    if (api.real) void api.layoutSet(vault.id, next).then(applyTeamView).catch((e) => toast({ text: t.common.saveFailed(errorText(e)), tone: "error" }));
  };
  const setMineView = (on: boolean) => {
    setMineViewState(on);
    try {
      localStorage.setItem(viewKey, on ? "mine" : "shared");
    } catch {
      /* private mode: for this run */
    }
    // The first time, mine starts as the shared grouping.
    if (on && !layout) saveLayout(layoutFrom(hosts, vaultGroups));
  };

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return hosts;
    return hosts.filter((h) => [h.label, h.address, h.user, h.group, ...h.tags].some((s) => s.toLowerCase().includes(q)));
  }, [hosts, query]);

  const groups = useMemo(
    () => (mine && layout ? listMine(filtered, layout, vaultGroups, !query.trim()) : listGroups(filtered, vaultGroups, !query.trim())),
    [filtered, vaultGroups, query, mine, layout],
  );
  const nameOfKey = (key: string | null) => (key === null ? null : groups.find((g) => g.key === key)?.name ?? key.slice(2));
  const drag = useDrag(groups, (d, drop) => {
    if (mine && layout) {
      if (d.kind === "host" && drop.kind === "host") saveLayout(mineMoveHost(layout, groups, d.id, drop.group, drop.before));
      if (d.kind === "group" && drop.kind === "group") saveLayout(mineMoveGroup(layout, d.key, drop.before));
      return;
    }
    if (d.kind === "host" && drop.kind === "host") update((w) => moveHost(w, vault.id, d.id, nameOfKey(drop.group)!, drop.before));
    if (d.kind === "group" && drop.kind === "group") update((w) => moveGroup(w, vault.id, d.name, nameOfKey(drop.before)));
  });
  // Everyone arranges their own grouping; the shared one is for those who manage it.
  const canArrange = mine || canEditGroups;

  const deleteGroup = (name: string) => {
    const before = ws;
    const record = ws.groups.find((g) => g.vaultId === vault.id && g.name === name);
    const members = ws.hosts.filter((h) => h.vaultId === vault.id && h.group === name).map((h) => h.id);
    update((w) => removeGroup(w, vault.id, name));
    toast({
      text: t.hosts.groups.removed(name),
      action: {
        label: t.hosts.restore,
        run: () =>
          update((w) => ({
            ...w,
            groups: record && !w.groups.some((g) => g.id === record.id) ? [...w.groups, record] : w.groups,
            hosts: w.hosts.map((h) => (members.includes(h.id) ? { ...h, group: before.hosts.find((x) => x.id === h.id)!.group } : h)),
          })),
      },
    });
  };

  const selected = hosts.find((h) => h.id === selectedId) ?? null;

  useEffect(() => setSelectedId(null), [vault.id]);

  const remove = (h: Host) => {
    const index = ws.hosts.findIndex((x) => x.id === h.id);
    update((w) => ({ ...w, hosts: w.hosts.filter((x) => x.id !== h.id) }));
    if (selectedId === h.id) setSelectedId(null);
    toast({
      text: t.hosts.removed(h.label),
      action: { label: t.hosts.restore, run: () => update((w) => ({ ...w, hosts: [...w.hosts.slice(0, index), h, ...w.hosts.slice(index)] })) },
    });
  };

  const duplicate = (h: Host) => {
    const copy: Host = { ...h, id: uid("h"), label: `${h.label}-copy`, lastConnected: undefined, hasPassword: false };
    update((w) => ({ ...w, hosts: [...w.hosts, copy] }));
    setSelectedId(copy.id);
    toast({ text: t.hosts.duplicated(h.label), tone: "success" });
  };

  const copyCmd = async (h: Host) => {
    const ok = await copyText(sshCommand(asLoggedIn(ws, h), ws.hosts.map((x) => asLoggedIn(ws, x))));
    toast(ok ? { text: t.hosts.sshCommandCopied, tone: "success" } : { text: t.common.copyFailed, tone: "error" });
  };

  const toggleGroup = (g: string) =>
    setCollapsed((s) => {
      const n = new Set(s);
      n.has(g) ? n.delete(g) : n.add(g);
      return n;
    });

  return (
    <div className={`split ${selected ? "with-panel" : ""}`}>
      <div className="split-main">
        <div className="page">
          <PageHeader title={t.hosts.title} count={hosts.length || undefined}>
            {vault.kind === "team" && hosts.length > 0 && (
              <Segmented
                value={mine ? "mine" : "shared"}
                onChange={(v) => setMineView(v === "mine")}
                options={[
                  { value: "shared", label: t.hosts.groups.shared },
                  { value: "mine", label: t.hosts.groups.mine },
                ]}
              />
            )}
            {mine && <Button icon={FolderPlus} onClick={() => setNaming({ from: null })}>{t.hosts.groups.add}</Button>}
            {hosts.length > 0 && (
              <label className="search">
                <Search size={14} />
                <input placeholder={t.common.search} value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setQuery("")} />
                {query && <IconButton icon={X} label={t.hosts.clearSearch} size={13} onClick={() => setQuery("")} />}
              </label>
            )}
            {(vault.kind === "personal" || hosts.length > 0 || (canCreate && canEditGroups)) && (
              <Menu
                trigger={(open, isOpen) => <IconButton icon={Ellipsis} label={t.hosts.more} className={isOpen ? "pressed" : ""} onClick={open} />}
                items={[
                  ...(vault.kind === "personal" ? [{ label: t.hosts.importSsh, icon: Download, onSelect: () => setImporting(true) }] : []),
                  ...(hosts.length > 0 ? [{ label: t.hosts.exportSsh, icon: Upload, onSelect: () => setExporting(hosts.map((h) => h.id)) }] : []),
                  ...(canCreate && canEditGroups && !mine ? [{ label: t.hosts.groups.add, icon: FolderPlus, onSelect: () => setNaming({ from: null }) }] : []),
                ]}
              />
            )}
            {canCreate && <Button variant="primary" icon={Plus} onClick={() => setCreating(true)}>{t.hosts.addHost}</Button>}
          </PageHeader>

          {hosts.length === 0 && (
            <Empty
              icon={Server}
              title={t.hosts.empty}
              action={
                canCreate ? (
                  <div className="row-buttons">
                    {vault.kind === "personal" && <Button icon={Download} onClick={() => setImporting(true)}>{t.hosts.importSsh}</Button>}
                    <Button variant="primary" icon={Plus} onClick={() => setCreating(true)}>{t.hosts.addHost}</Button>
                  </div>
                ) : undefined
              }
            />
          )}
          {hosts.length > 0 && filtered.length === 0 && <Empty icon={Search} title={t.hosts.noMatch(query)} />}

          {groups.map((listed) => {
            const { key, name: group, hosts: list } = listed;
            const isCollapsed = collapsed.has(key) && !query;
            const dropHere = drag.drop?.kind === "host" && drag.drop.group === key;
            const prefs = group ? ws.groupPrefs.find((p) => p.key === groupPrefsKey(vault, listed)) : undefined;
            const sharedProxy = !listed.mine && listed.record?.proxyId ? ws.proxies.find((p) => p.id === listed.record!.proxyId) : undefined;
            const proxyName = prefs?.proxy ? ws.proxies.find((p) => p.id === prefs.proxy!.id && p.vaultId === prefs.proxy!.vault)?.name : sharedProxy?.name;
            // Groups of mine are this person's to change; shared ones, a manager's.
            const editable = !!group && (listed.mine ? true : !mine && canEditGroups);
            return (
              <section
                className={`group ${drag.drop?.kind === "group" && drag.drop.before === key ? "drop-before" : ""} ${dropHere && (isCollapsed || !list.length) ? "drop-into" : ""}`}
                key={key}
                data-group={key}
              >
                <div className="group-bar">
                  <button
                    className={`group-head ${group && canArrange ? "draggable" : ""}`}
                    onClick={() => !drag.justDragged() && toggleGroup(key)}
                    onPointerDown={(e) => group && canArrange && drag.start(e, { kind: "group", name: group, key, label: group })}
                    aria-expanded={!isCollapsed}
                  >
                    <ChevronRight size={14} className={`chev ${isCollapsed ? "" : "open"}`} />
                    {groupLabel(group)}
                    <span className="count">{list.length}</span>
                    {proxyName && (
                      <span className="group-badge" title={proxyName}>
                        <Shield size={11} /> {proxyName}
                      </span>
                    )}
                    {(prefs?.auth || prefs?.user) && (
                      <span className="group-badge">
                        <KeyRound size={11} /> {prefs.user ?? ""}
                      </span>
                    )}
                  </button>
                  {group && (
                    <Menu
                      trigger={(open, isOpen) => <IconButton icon={Ellipsis} label={t.hosts.groups.menu} size={13} className={`group-menu ${isOpen ? "pressed" : ""}`} onClick={open} />}
                      items={[
                        { label: t.hosts.groups.settings, icon: Settings2, onSelect: () => setSettingsOf(listed) },
                        ...(editable
                          ? [
                              { label: t.hosts.groups.rename, icon: Pencil, onSelect: () => setNaming({ from: group, mine: listed.mine?.id }) },
                              {
                                label: t.common.delete,
                                icon: Trash2,
                                danger: true,
                                onSelect: () => {
                                  if (!listed.mine || !layout) return deleteGroup(group);
                                  const before = layout;
                                  saveLayout(mineRemove(layout, listed.mine.id));
                                  toast({ text: t.hosts.groups.removedMine(group), action: { label: t.hosts.restore, run: () => saveLayout(before) } });
                                },
                              },
                            ]
                          : []),
                      ]}
                    />
                  )}
                </div>
                {!isCollapsed && !list.length && <div className={`group-empty ${dropHere ? "over" : ""}`}>{t.hosts.groups.dropHere}</div>}
                {!isCollapsed && list.length > 0 && (
                  <div className={`rows ${dropHere && drag.drop?.kind === "host" && drag.drop.before === null ? "drop-end" : ""}`}>
                    {list.map((h) => (
                      <div
                        key={h.id}
                        role="button"
                        tabIndex={0}
                        data-host={h.id}
                        className={`host-row ${h.id === selectedId ? "selected" : ""} ${drag.dragging === h.id ? "dragged" : ""} ${dropHere && drag.drop?.kind === "host" && drag.drop.before === h.id ? "drop-before" : ""}`}
                        onPointerDown={(e) => (mine || canMoveHost(h)) && !query && drag.start(e, { kind: "host", id: h.id, label: h.label })}
                        onClick={() => !drag.justDragged() && setSelectedId(h.id === selectedId ? null : h.id)}
                        onDoubleClick={() => connect(h.id)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") connect(h.id);
                          if (hit(e, "removeHost") && atLeast(h.access, "edit")) remove(h);
                        }}
                      >
                        <HostIcon host={h} />
                        <div className="host-main">
                          <span className="host-label">{h.label}</span>
                          <span className="host-addr">
                            {h.user}@{h.address}
                            {h.port !== 22 && `:${h.port}`}
                          </span>
                        </div>
                        <div className="host-tags">
                          {h.tags.map((tag) => <span className="tag" key={tag}>{tag}</span>)}
                        </div>
                        {conflicted.has(h.id) ? (
                          <button
                            className="chip warn chip-btn host-last-chip"
                            onClick={(e) => {
                              e.stopPropagation();
                              resolveConflicts();
                            }}
                          >
                            <TriangleAlert size={12} /> {t.hosts.conflict}
                          </button>
                        ) : (
                          <span className="host-last">{formatAgo(h.lastConnected)}</span>
                        )}
                        <div className="row-actions">
                          <Button
                            size="sm"
                            variant="primary"
                            icon={SquareTerminal}
                            onClick={(e) => {
                              e.stopPropagation();
                              connect(h.id);
                            }}
                          >
                            {t.common.connect}
                          </Button>
                          <Menu
                            trigger={(toggle, open) => <IconButton icon={Ellipsis} label={t.hosts.more} className={open ? "pressed" : ""} onClick={toggle} />}
                            items={[
                              { label: t.hosts.files, icon: FolderOpen, onSelect: () => openFiles(h.id) },
                              { label: t.hosts.copySshCommand, icon: Terminal, onSelect: () => copyCmd(h) },
                              { label: t.hosts.exportSsh, icon: Upload, onSelect: () => setExporting([h.id]) },
                              ...(vault.kind === "team" && api.real ? [{ label: t.teams.history.history, icon: History, onSelect: () => setHistoryOf(h) }] : []),
                              ...(canCreate ? [{ label: t.hosts.duplicate, icon: CopyPlus, onSelect: () => duplicate(h) }] : []),
                              ...(atLeast(h.access, "edit") ? [{ label: t.common.delete, icon: Trash2, danger: true, hint: keysOf("removeHost"), onSelect: () => remove(h) }] : []),
                            ]}
                          />
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </section>
            );
          })}
        </div>
      </div>

      {selected && (
        <HostPanel key={[selected.id, selected.address, selected.port, selected.user, selected.label, selected.group, selected.access, selected.forwardAgent, JSON.stringify(selected.own)].join("|")} host={selected} groups={groupNames} onClose={() => setSelectedId(null)} onDelete={() => remove(selected)} onCopy={() => copyCmd(selected)} />
      )}

      {creating && <NewHostDialog groups={groupNames} onClose={() => setCreating(false)} onCreated={(h) => setSelectedId(h.id)} />}
      {settingsOf && <GroupSettingsDialog group={settingsOf} mine={mine} onClose={() => setSettingsOf(null)} />}
      {naming && (
        <GroupNameDialog
          initial={naming.from ?? ""}
          taken={(mine && layout ? layout.groups.map((g) => g.name) : groupNames).filter((g) => g !== naming.from)}
          onClose={() => setNaming(null)}
          onSave={(name) => {
            if (mine && layout) saveLayout(naming.mine ? mineRename(layout, naming.mine, name) : mineAdd(layout, name));
            else if (naming.from === null) update((w) => addGroup(w, vault.id, name));
            else if (name !== naming.from) update((w) => renameGroup(w, vault.id, naming.from!, name));
            toast({ text: naming.from === null ? t.hosts.groups.added(name) : t.hosts.groups.renamed(name), tone: "success" });
            setNaming(null);
          }}
        />
      )}
      {drag.ghost}
      {importing && <ImportDialog onClose={() => setImporting(false)} />}
      {exporting && <ExportDialog hosts={hosts} initial={exporting} onClose={() => setExporting(null)} />}
      {historyOf && <RecordHistoryDialog vault={vault} record={historyOf.id} name={historyOf.label} onClose={() => setHistoryOf(null)} />}
    </div>
  );
}

/* ---------- dragging hosts and groups ---------- */

type Dragged = { kind: "host"; id: string; label: string } | { kind: "group"; name: string; key: string; label: string };
/** Where a drag lands: before a host of a group (by key; null: last), or before a group (by key; null: last). */
type Drop = { kind: "host"; group: string; before: string | null } | { kind: "group"; before: string | null };

/**
 * Dragging with the pointer (the webview's own drag and drop is taken by the window for files).
 * A press becomes a drag after a few pixels; a drag doesn't count as a click.
 */
function useDrag(groups: ListedGroup[], onDrop: (d: Dragged, drop: Drop) => void) {
  const [state, setState] = useState<{ what: Dragged; x: number; y: number; drop: Drop | null } | null>(null);
  const press = useRef<{ what: Dragged; x: number; y: number } | null>(null);
  const dragged = useRef(0);
  // The drag as last set, for the drop: updaters must stay pure, and dropping changes the app.
  const current = useRef(state);
  const latest = useRef({ groups, onDrop });
  latest.current = { groups, onDrop };

  /** Where the pointer is, for what is dragged. */
  const dropAt = (what: Dragged, x: number, y: number): Drop | null => {
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    const section = el?.closest<HTMLElement>("[data-group]");
    if (!section) return null;
    const group = section.dataset.group ?? "";
    const list = latest.current.groups;
    if (what.kind === "group") {
      const rect = section.getBoundingClientRect();
      const i = list.findIndex((g) => g.key === group);
      const next = (k: number) => list.slice(k).find((g) => g.name && g.key !== what.key)?.key ?? null;
      // Hosts without a group stay last.
      if (!list[i]?.name) return { kind: "group", before: null };
      return { kind: "group", before: y < rect.top + rect.height / 2 ? next(i) : next(i + 1) };
    }
    const row = el?.closest<HTMLElement>("[data-host]");
    if (row) {
      const rect = row.getBoundingClientRect();
      const hosts = list.find((g) => g.key === group)?.hosts ?? [];
      const i = hosts.findIndex((h) => h.id === row.dataset.host);
      const after = hosts.slice(i + 1).find((h) => h.id !== what.id)?.id ?? null;
      const here = row.dataset.host === what.id ? after : row.dataset.host!;
      return { kind: "host", group, before: y < rect.top + rect.height / 2 ? here : after };
    }
    return { kind: "host", group, before: null };
  };

  useEffect(() => {
    const move = (e: PointerEvent) => {
      const p = press.current;
      if (!p) return;
      if (!state && Math.hypot(e.clientX - p.x, e.clientY - p.y) < 6) return;
      e.preventDefault();
      current.current = { what: p.what, x: e.clientX, y: e.clientY, drop: dropAt(p.what, e.clientX, e.clientY) };
      setState(current.current);
    };
    const up = () => {
      const p = press.current;
      const s = current.current;
      press.current = null;
      current.current = null;
      setState(null);
      if (s && p) {
        dragged.current = Date.now();
        if (s.drop) latest.current.onDrop(s.what, s.drop);
      }
    };
    const cancel = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !press.current) return;
      press.current = null;
      current.current = null;
      setState(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", cancel);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", cancel);
    };
  }, [state]);

  return {
    start: (e: React.PointerEvent, what: Dragged) => {
      // Buttons and menus in a row are pressed, not dragged.
      if (e.button !== 0 || (e.target as HTMLElement).closest(".row-actions, .group-menu, input")) return;
      press.current = { what, x: e.clientX, y: e.clientY };
    },
    drop: state?.drop ?? null,
    dragging: state?.what.kind === "host" ? state.what.id : state?.what.kind === "group" ? state.what.key : null,
    /** A click right after a drag is the drag's end. */
    justDragged: () => Date.now() - dragged.current < 250,
    ghost: state ? (
      <div className="drag-ghost" style={{ left: state.x + 14, top: state.y + 10 }}>
        {state.what.kind === "group" ? <FolderOpen size={13} /> : <Server size={13} />}
        {state.what.label}
      </div>
    ) : null,
  };
}

function GroupNameDialog({ initial, taken, onClose, onSave }: { initial: string; taken: string[]; onClose: () => void; onSave: (name: string) => void }) {
  const t = useT();
  const [name, setName] = useState(initial);
  const [touched, setTouched] = useState(false);
  const clean = name.trim();
  const error = !clean ? t.hosts.groups.nameRequired : taken.some((g) => g.toLowerCase() === clean.toLowerCase()) ? t.hosts.groups.exists : null;
  return (
    <Modal
      title={initial ? t.hosts.groups.rename : t.hosts.groups.add}
      onClose={onClose}
      onSubmit={() => {
        setTouched(true);
        if (!error) onSave(clean);
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary">{initial ? t.common.save : t.common.create}</Button>
        </>
      }
    >
      <Field label={t.hosts.groups.name} error={touched ? error : null}>
        <input className={`input ${touched && error ? "invalid" : ""}`} value={name} onChange={(e) => setName(e.target.value)} placeholder="Production" />
      </Field>
    </Modal>
  );
}

function HostPanel({ host, groups, onClose, onDelete, onCopy }: { host: Host; groups: string[]; onClose: () => void; onDelete: () => void; onCopy: () => void }) {
  const t = useT();
  const { ws, vault, update, toast, connect, openFiles, resolveConflicts } = useStore();
  // A team host's shared key must be a team key: teammates don't have your personal ones.
  const keys = ws.keys.filter((k) => k.vaultId === vault.id);
  const conflict = vault.conflicts.find((c) => c.coll === "host" && c.record === host.id);
  const canEdit = atLeast(host.access, "edit");
  const shared = atLeast(host.access, "secret");
  const team = vault.kind === "team";
  const initial = draftFromHost(host);
  const { draft, set, setDraft, touched, setTouched } = useHostDraft(initial);
  const dirty = JSON.stringify(draft) !== JSON.stringify(initial);
  const invalid = draftInvalid(draft);
  const jumpHosts = ws.hosts.filter((h) => h.vaultId === vault.id && h.id !== host.id);

  const save = () => {
    setTouched(true);
    if (invalid) return;
    const typed = draft.authType === "password" && draft.password ? draft.password : null;
    update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? withPassword({ ...h, ...applyDraft(draft) }, typed) : h)) }));
    toast({ text: t.hosts.panel.saved, tone: "success" });
  };

  const forgetPassword = async () => {
    try {
      await api.hostSetPassword(host, null);
      update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, hasPassword: false } : h)) }));
      toast({ text: t.hosts.panel.passwordForgotten, tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  return (
    <aside className="panel">
      <div className="panel-head">
        <HostIcon host={host} size={36} />
        <div className="panel-title">
          <h2>{host.label}</h2>
          {host.online !== undefined && <span className={`status-text ${host.online ? "on" : "off"}`}>{host.online ? t.hosts.panel.online : t.hosts.panel.offline}</span>}
        </div>
        <IconButton icon={X} label={t.common.close} tipSide="left" onClick={onClose} />
      </div>

      {conflict && (
        <button className="banner warn banner-btn" onClick={() => resolveConflicts()}>
          <TriangleAlert size={15} />
          <span className="grow">{conflict.kind === "deleted" ? t.hosts.panel.conflictDeleted : t.hosts.panel.conflictEdited}</span>
          <span className="sidebar-alert-action">{t.hosts.panel.resolve}</span>
        </button>
      )}

      <div className="panel-connect">
        <Button variant="primary" icon={SquareTerminal} onClick={() => connect(host.id)}>
          {t.common.connect}
        </Button>
        <Button icon={FolderOpen} onClick={() => openFiles(host.id)}>
          {t.hosts.files}
        </Button>
      </div>

      <form
        className="panel-form"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
        onKeyDown={(e) => e.key === "Escape" && dirty && (e.stopPropagation(), setDraft(initial))}
      >
        {canEdit ? (
          <HostFields draft={draft} set={set} keys={keys} groups={groups} showErrors={touched} passwordSaved={host.hasPassword} onForgetPassword={forgetPassword} savedPassword={host.hasPassword ? <SavedPassword host={host} /> : null} jumpHosts={jumpHosts} />
        ) : (
          <HostDetails host={host} shared={shared} keyName={keys.find((k) => host.auth.type === "key" && k.id === host.auth.keyId)?.name} />
        )}
        {dirty && (
          <div className="panel-save">
            <Button type="button" variant="ghost" onClick={() => setDraft(initial)}>{t.hosts.panel.discard}</Button>
            <Button type="submit" variant="primary">{t.common.save}</Button>
          </div>
        )}
      </form>

      {canEdit && <FromGroupsSection host={host} />}
      {team && <OwnLoginSection host={host} />}

      <div className="grow" />

      <dl className="meta">
        {host.lastConnected && (
          <>
            <dt>{t.hosts.panel.lastConnected}</dt>
            <dd>{formatAgo(host.lastConnected)}</dd>
          </>
        )}
      </dl>

      <div className="panel-foot">
        <Button variant="ghost" icon={Terminal} onClick={onCopy}>{t.hosts.panel.copySsh}</Button>
        {canEdit && <Button variant="ghost" icon={Trash2} className="danger-text" onClick={onDelete}>{t.common.delete}</Button>}
      </div>
    </aside>
  );
}

/** What a member who may not edit the host sees of it. */
/** What a host takes from its groups when connecting, as text. */
function useFromGroups(host: Host) {
  const t = useT();
  const { ws } = useStore();
  const from = fromGroups(ws, host);
  const proxy = from.proxy && ws.proxies.find((x) => x.id === from.proxy!.id && x.vaultId === from.proxy!.vault);
  const login = from.login?.prefs;
  const auth = login?.auth;
  return {
    from,
    proxyText: proxy ? `${proxy.name} (${t.proxies.types[proxy.kind.type]})` : from.proxy ? "?" : null,
    user: login?.user ?? null,
    authText: auth ? (auth.type === "key" ? ws.keys.find((k) => k.id === auth.keyId)?.name ?? t.common.key : auth.type === "agent" ? "ssh-agent" : t.common.password) : null,
    note: (group: string) => <span className="from-group">{t.hosts.details.fromGroup(group)}</span>,
  };
}

/** Under the form of a host: what its groups put over it when connecting. */
function FromGroupsSection({ host }: { host: Host }) {
  const t = useT();
  const { from, proxyText, user, authText, note } = useFromGroups(host);
  if (!proxyText && !user && !authText) return null;
  return (
    <section className="from-groups">
      <h3 className="section-title">{t.hosts.details.fromGroups}</h3>
      <dl className="meta details">
        {proxyText && (
          <>
            <dt>{t.hosts.details.via}</dt>
            <dd>{proxyText}{note(from.proxy!.group)}</dd>
          </>
        )}
        {user && (
          <>
            <dt>{t.hosts.form.user}</dt>
            <dd><span className="mono">{user}</span>{note(from.login!.group)}</dd>
          </>
        )}
        {authText && (
          <>
            <dt>{t.hosts.form.auth}</dt>
            <dd>{authText}{note(from.login!.group)}</dd>
          </>
        )}
      </dl>
    </section>
  );
}

function HostDetails({ host, shared, keyName }: { host: Host; shared: boolean; keyName?: string }) {
  const t = useT();
  const { ws, update } = useStore();
  // Forwarding is each person's own choice, so it can be changed without edit access.
  const setForward = (forwardAgent: boolean) => update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, forwardAgent } : h)) }));
  const p = host.proxy;
  const { from, proxyText, user: groupUser, authText: loginAuth, note: fromNote } = useFromGroups(host);
  const login = groupUser ? { user: groupUser } : null;
  const saved = host.proxyId ? ws.proxies.find((x) => x.id === host.proxyId) : undefined;
  const via =
    proxyText ??
    (saved
      ? `${saved.name} (${t.proxies.types[saved.kind.type]})`
      : p?.type === "jump"
        ? ws.hosts.find((h) => h.id === p.hostId)?.label ?? "?"
        : p
          ? `${p.type === "socks5" ? "SOCKS5" : "HTTP"} ${p.address}:${p.port}`
          : null);
  return (
    <>
      <dl className="meta details">
        <dt>{t.hosts.form.address}</dt>
        <dd className="mono">{host.address}{host.port !== 22 && `:${host.port}`}</dd>
        <dt>{t.hosts.form.user}</dt>
        <dd>
          <span className="mono">{login?.user ?? host.user}</span>
          {login?.user && fromNote(from.login!.group)}
        </dd>
        {loginAuth ? (
          <>
            <dt>{t.hosts.form.auth}</dt>
            <dd>
              {loginAuth}
              {fromNote(from.login!.group)}
            </dd>
          </>
        ) : (
          shared && (
            <>
              <dt>{t.hosts.form.auth}</dt>
              <dd>{authLabel(host, keyName)}</dd>
            </>
          )
        )}
        {!loginAuth && shared && host.auth.type === "password" && host.hasPassword && (
          <>
            <dt>{t.common.password}</dt>
            <dd><SavedPassword host={host} /></dd>
          </>
        )}
        {via && (
          <>
            <dt>{t.hosts.details.via}</dt>
            <dd>
              <span className="mono">{via}</span>
              {from.proxy && fromNote(from.proxy.group)}
            </dd>
          </>
        )}
        <dt>{t.hosts.form.group}</dt>
        <dd>{groupLabel(host.group)}</dd>
        {host.tags.length > 0 && (
          <>
            <dt>{t.hosts.form.tags}</dt>
            <dd>{host.tags.join(", ")}</dd>
          </>
        )}
      </dl>
      <div className="field">
        <Checkbox checked={!!host.forwardAgent} onChange={setForward}>{t.hosts.form.forwardAgent}</Checkbox>
        {host.forwardAgent && <span className="small warn-text">{t.hosts.form.forwardAgentRisk}</span>}
      </div>
    </>
  );
}

function NewHostDialog({ groups, onClose, onCreated }: { groups: string[]; onClose: () => void; onCreated: (h: Host) => void }) {
  const t = useT();
  const { ws, vault, update, toast, connect } = useStore();
  const keys = ws.keys.filter((k) => k.vaultId === vault.id);
  const { draft, set, touched, setTouched } = useHostDraft(emptyDraft(keys, groups[0] ?? ""));
  const jumpHosts = ws.hosts.filter((h) => h.vaultId === vault.id);

  const submit = () => {
    setTouched(true);
    if (draftInvalid(draft)) return;
    const host: Host = withPassword(
      {
        id: uid("h"),
        vaultId: vault.id,
        ...applyDraft(draft),
        hasPassword: false,
        ...(api.usesCore(vault.id) ? {} : { os: "ubuntu" as const, online: true }),
      },
      draft.authType === "password" && draft.password ? draft.password : null,
    );
    update((w) => ({ ...w, hosts: [...w.hosts, host] }));
    onCreated(host);
    toast({ text: t.hosts.newHost.added(host.label), tone: "success", action: { label: t.common.connect, run: () => connect(host.id) } });
    onClose();
  };

  return (
    <Modal
      title={t.hosts.newHost.title}
      width={480}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary">{t.common.add}</Button>
        </>
      }
    >
      <HostFields draft={draft} set={set} keys={keys} groups={groups} showErrors={touched} jumpHosts={jumpHosts} />
    </Modal>
  );
}

/** Attaches a typed password: detour-core receives it with the next vault save, demo vaults keep it in memory. */
function withPassword(h: Host, password: string | null): Host {
  if (!password) return h;
  if (api.usesCore(h.vaultId)) return { ...h, newPassword: password, hasPassword: true };
  void api.hostSetPassword(h, password);
  return { ...h, hasPassword: true };
}
