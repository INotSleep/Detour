import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ArrowLeftRight, Code, CornerDownLeft, KeyRound, Lock, Search, Settings, Users, type LucideIcon } from "lucide-react";
import { api } from "../lib/api";
import { useStore, type Section } from "../lib/store";
import { HostIcon } from "./common";
import type { Host } from "../lib/types";

type Item =
  | { kind: "host"; id: string; host: Host; vaultName: string | null }
  | { kind: "go"; id: string; label: string; icon: LucideIcon; section: Section }
  | { kind: "lock"; id: string; label: string; icon: LucideIcon };

const goItems: { label: string; icon: LucideIcon; section: Section }[] = [
  { label: "Ключи", icon: KeyRound, section: "keys" },
  { label: "Сниппеты", icon: Code, section: "snippets" },
  { label: "Туннели", icon: ArrowLeftRight, section: "tunnels" },
  { label: "Команда", icon: Users, section: "team" },
  { label: "Настройки", icon: Settings, section: "settings" },
];

export function CommandPalette({ onClose }: { onClose: () => void }) {
  const { ws, vault, connect, navigate, lock } = useStore();
  const [q, setQ] = useState("");
  const [index, setIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const items = useMemo<Item[]>(() => {
    const query = q.trim().toLowerCase();
    const match = (...s: string[]) => !query || s.some((x) => x.toLowerCase().includes(query));
    const hosts = [...ws.hosts]
      .sort((a, b) => Number(b.vaultId === vault.id) - Number(a.vaultId === vault.id))
      .filter((h) => match(h.label, h.address, h.user, ...h.tags))
      .map<Item>((h) => ({ kind: "host", id: h.id, host: h, vaultName: h.vaultId === vault.id ? null : ws.vaults.find((v) => v.id === h.vaultId)?.name ?? null }));
    const go = goItems.filter((g) => match(g.label)).map<Item>((g) => ({ kind: "go", id: g.section, ...g }));
    const actions: Item[] = api.real && match("заблокировать", "lock") ? [{ kind: "lock", id: "lock", label: "Заблокировать", icon: Lock }] : [];
    return [...hosts, ...go, ...actions];
  }, [q, ws, vault.id]);

  useEffect(() => setIndex(0), [q]);

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const run = (it: Item | undefined) => {
    if (!it) return;
    onClose();
    if (it.kind === "host") connect(it.host.id);
    else if (it.kind === "go") navigate(it.section);
    else void lock();
  };

  const hostCount = items.filter((i) => i.kind === "host").length;

  return createPortal(
    <div className="modal-backdrop palette-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" role="dialog" aria-label="Быстрое подключение">
        <div className="palette-input">
          <Search size={16} />
          <input
            autoFocus
            value={q}
            placeholder="Хост, адрес или раздел"
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) => Math.min(items.length - 1, i + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(0, i - 1));
              } else if (e.key === "Enter") {
                run(items[index]);
              } else if (e.key === "Escape") {
                onClose();
              }
            }}
          />
        </div>
        <div className="palette-list" ref={listRef}>
          {items.length === 0 && <div className="palette-empty">Ничего не найдено</div>}
          {items.map((it, i) => (
            <div key={`${it.kind}-${it.id}`}>
              {i === 0 && it.kind === "host" && <div className="palette-section">Хосты</div>}
              {i === hostCount && it.kind === "go" && <div className="palette-section">Перейти</div>}
              <button data-index={i} className={`palette-item ${i === index ? "active" : ""}`} onMouseMove={() => setIndex(i)} onClick={() => run(it)}>
                {it.kind === "host" ? (
                  <>
                    <HostIcon host={it.host} size={24} />
                    <span className="strong">{it.host.label}</span>
                    <span className="muted mono small">{it.host.user}@{it.host.address}</span>
                    <span className="grow" />
                    {it.vaultName && <span className="chip">{it.vaultName}</span>}
                  </>
                ) : (
                  <>
                    <span className="palette-go-icon"><it.icon size={14} /></span>
                    <span>{it.label}</span>
                    <span className="grow" />
                  </>
                )}
                {i === index && <CornerDownLeft size={13} className="muted" />}
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>,
    document.body,
  );
}
