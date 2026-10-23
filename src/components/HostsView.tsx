import { useEffect, useMemo, useState } from "react";
import { ChevronRight, CopyPlus, Ellipsis, Plus, Search, Server, SquareTerminal, Terminal, Trash2, TriangleAlert, X } from "lucide-react";
import { copyText, sshCommand, uid, useStore } from "../lib/store";
import type { Host } from "../lib/types";
import { HostIcon } from "./common";
import { Button, Empty, IconButton, Menu, Modal, PageHeader } from "./ui";
import { HostFields, applyDraft, draftErrors, draftFromHost, emptyDraft, useHostDraft } from "./HostForm";

export function HostsView() {
  const { ws, vault, update, toast, connect, resolveConflicts } = useStore();
  const conflicted = new Set(vault.conflicts.map((c) => c.hostId));
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [creating, setCreating] = useState(false);

  const hosts = ws.hosts.filter((h) => h.vaultId === vault.id);
  const groupNames = [...new Set(hosts.map((h) => h.group))];

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return hosts;
    return hosts.filter((h) => [h.label, h.address, h.user, h.group, ...h.tags].some((s) => s.toLowerCase().includes(q)));
  }, [hosts, query]);

  const groups = useMemo(() => {
    const map = new Map<string, Host[]>();
    for (const h of filtered) map.set(h.group, [...(map.get(h.group) ?? []), h]);
    return [...map.entries()];
  }, [filtered]);

  const selected = hosts.find((h) => h.id === selectedId) ?? null;

  useEffect(() => setSelectedId(null), [vault.id]);

  const remove = (h: Host) => {
    const index = ws.hosts.findIndex((x) => x.id === h.id);
    update((w) => ({ ...w, hosts: w.hosts.filter((x) => x.id !== h.id) }));
    if (selectedId === h.id) setSelectedId(null);
    toast({
      text: `«${h.label}» удалён`,
      action: { label: "Вернуть", run: () => update((w) => ({ ...w, hosts: [...w.hosts.slice(0, index), h, ...w.hosts.slice(index)] })) },
    });
  };

  const duplicate = (h: Host) => {
    const copy: Host = { ...h, id: uid("h"), label: `${h.label}-copy`, lastConnected: undefined };
    update((w) => ({ ...w, hosts: [...w.hosts, copy] }));
    setSelectedId(copy.id);
    toast({ text: `Создана копия «${h.label}»`, tone: "success" });
  };

  const copyCmd = async (h: Host) => {
    const ok = await copyText(sshCommand(h));
    toast(ok ? { text: "Команда ssh скопирована", tone: "success" } : { text: "Не удалось скопировать", tone: "error" });
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
          <PageHeader title="Хосты" count={hosts.length || undefined}>
            {hosts.length > 0 && (
              <label className="search">
                <Search size={14} />
                <input placeholder="Поиск" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setQuery("")} />
                {query && <IconButton icon={X} label="Очистить" size={13} onClick={() => setQuery("")} />}
              </label>
            )}
            <Button variant="primary" icon={Plus} onClick={() => setCreating(true)}>Добавить хост</Button>
          </PageHeader>

          {hosts.length === 0 && (
            <Empty icon={Server} title="Нет хостов" action={<Button variant="primary" icon={Plus} onClick={() => setCreating(true)}>Добавить хост</Button>} />
          )}
          {hosts.length > 0 && filtered.length === 0 && <Empty icon={Search} title={`Ничего не найдено по «${query}»`} />}

          {groups.map(([group, list]) => {
            const isCollapsed = collapsed.has(group) && !query;
            return (
              <section className="group" key={group}>
                <button className="group-head" onClick={() => toggleGroup(group)} aria-expanded={!isCollapsed}>
                  <ChevronRight size={14} className={`chev ${isCollapsed ? "" : "open"}`} />
                  {group}
                  <span className="count">{list.length}</span>
                </button>
                {!isCollapsed && (
                  <div className="rows">
                    {list.map((h) => (
                      <div
                        key={h.id}
                        role="button"
                        tabIndex={0}
                        className={`host-row ${h.id === selectedId ? "selected" : ""}`}
                        onClick={() => setSelectedId(h.id === selectedId ? null : h.id)}
                        onDoubleClick={() => connect(h.id)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") connect(h.id);
                          if (e.key === "Delete") remove(h);
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
                          {h.tags.map((t) => <span className="tag" key={t}>{t}</span>)}
                        </div>
                        {conflicted.has(h.id) ? (
                          <button
                            className="chip warn chip-btn host-last-chip"
                            onClick={(e) => {
                              e.stopPropagation();
                              resolveConflicts();
                            }}
                          >
                            <TriangleAlert size={12} /> Конфликт
                          </button>
                        ) : (
                          <span className="host-last">{h.lastConnected ?? ""}</span>
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
                            Подключиться
                          </Button>
                          <Menu
                            trigger={(toggle, open) => <IconButton icon={Ellipsis} label="Ещё" className={open ? "pressed" : ""} onClick={toggle} />}
                            items={[
                              { label: "Копировать команду ssh", icon: Terminal, onSelect: () => copyCmd(h) },
                              { label: "Дублировать", icon: CopyPlus, onSelect: () => duplicate(h) },
                              { label: "Удалить", icon: Trash2, danger: true, hint: "Del", onSelect: () => remove(h) },
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
        <HostPanel key={[selected.id, selected.address, selected.port, selected.user, selected.label, selected.group].join("|")} host={selected} groups={groupNames} onClose={() => setSelectedId(null)} onDelete={() => remove(selected)} onCopy={() => copyCmd(selected)} />
      )}

      {creating && <NewHostDialog groups={groupNames} onClose={() => setCreating(false)} onCreated={(h) => setSelectedId(h.id)} />}
    </div>
  );
}

function HostPanel({ host, groups, onClose, onDelete, onCopy }: { host: Host; groups: string[]; onClose: () => void; onDelete: () => void; onCopy: () => void }) {
  const { ws, vault, update, toast, connect, resolveConflicts } = useStore();
  const keys = ws.keys.filter((k) => k.vaultId === vault.id || k.vaultId === "personal");
  const conflict = vault.conflicts.find((c) => c.hostId === host.id);
  const conflictBy = conflict && ws.members.find((m) => m.id === conflict.theirsBy)?.name.split(" ")[0];
  const initial = draftFromHost(host);
  const { draft, set, setDraft, touched, setTouched } = useHostDraft(initial);
  const dirty = JSON.stringify(draft) !== JSON.stringify(initial);
  const errors = draftErrors(draft);
  const invalid = !!(errors.address || errors.port);
  const addedBy = ws.members.find((m) => m.id === host.addedBy);

  const save = () => {
    setTouched(true);
    if (invalid) return;
    update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, ...applyDraft(draft) } : h)) }));
    toast({ text: "Сохранено", tone: "success" });
  };

  return (
    <aside className="panel">
      <div className="panel-head">
        <HostIcon host={host} size={36} />
        <div className="panel-title">
          <h2>{host.label}</h2>
          <span className={`status-text ${host.online ? "on" : "off"}`}>{host.online ? "В сети" : "Недоступен"}</span>
        </div>
        <IconButton icon={X} label="Закрыть" tipSide="left" onClick={onClose} />
      </div>

      {conflict && (
        <button className="banner warn banner-btn" onClick={() => resolveConflicts()}>
          <TriangleAlert size={15} />
          <span className="grow">Изменён у вас и у {conflictBy}</span>
          <span className="sidebar-alert-action">Решить</span>
        </button>
      )}

      <Button variant="primary" icon={SquareTerminal} className="block" onClick={() => connect(host.id)}>
        Подключиться
      </Button>

      <form
        className="panel-form"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
        onKeyDown={(e) => e.key === "Escape" && dirty && (e.stopPropagation(), setDraft(initial))}
      >
        <HostFields draft={draft} set={set} keys={keys} groups={groups} showErrors={touched} />
        {dirty && (
          <div className="panel-save">
            <Button type="button" variant="ghost" onClick={() => setDraft(initial)}>Отменить</Button>
            <Button type="submit" variant="primary">Сохранить</Button>
          </div>
        )}
      </form>

      <div className="grow" />

      <dl className="meta">
        {host.lastConnected && (
          <>
            <dt>Последний вход</dt>
            <dd>{host.lastConnected}</dd>
          </>
        )}
        {addedBy && (
          <>
            <dt>Добавил</dt>
            <dd>{addedBy.isYou ? "Вы" : addedBy.name}</dd>
          </>
        )}
      </dl>

      <div className="panel-foot">
        <Button variant="ghost" icon={Terminal} onClick={onCopy}>Копировать ssh</Button>
        <Button variant="ghost" icon={Trash2} className="danger-text" onClick={onDelete}>Удалить</Button>
      </div>
    </aside>
  );
}

function NewHostDialog({ groups, onClose, onCreated }: { groups: string[]; onClose: () => void; onCreated: (h: Host) => void }) {
  const { ws, vault, update, toast, connect } = useStore();
  const keys = ws.keys.filter((k) => k.vaultId === vault.id || k.vaultId === "personal");
  const { draft, set, touched, setTouched } = useHostDraft(emptyDraft(keys, groups[0] ?? ""));
  const errors = draftErrors(draft);

  const submit = () => {
    setTouched(true);
    if (errors.address || errors.port) return;
    const host: Host = {
      id: uid("h"),
      vaultId: vault.id,
      ...applyDraft(draft),
      os: "ubuntu",
      online: true,
      addedBy: ws.members.find((m) => m.vaultId === vault.id && m.isYou)?.id,
    };
    update((w) => ({ ...w, hosts: [...w.hosts, host] }));
    onCreated(host);
    toast({ text: `«${host.label}» добавлен`, tone: "success", action: { label: "Подключиться", run: () => connect(host.id) } });
    onClose();
  };

  return (
    <Modal
      title="Новый хост"
      width={480}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary">Добавить</Button>
        </>
      }
    >
      <HostFields draft={draft} set={set} keys={keys} groups={groups} showErrors={touched} />
    </Modal>
  );
}
