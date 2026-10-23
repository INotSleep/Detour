import { useEffect, useRef, useState } from "react";
import { ArrowLeftRight, Check, ChevronsUpDown, Code, KeyRound, LogIn, Plus, Search, Server, Settings, TriangleAlert, Users } from "lucide-react";
import { useT } from "../i18n";
import { formatAgo, useStore, type Section } from "../lib/store";
import type { Vault } from "../lib/types";
import { VaultAvatar } from "./common";
import { Kbd } from "./ui";
import { CreateTeamDialog, JoinTeamDialog } from "./TeamDialogs";
import { syncErrorText } from "./SyncSettings";

const nav: { id: Exclude<Section, "settings">; icon: typeof Server }[] = [
  { id: "hosts", icon: Server },
  { id: "keys", icon: KeyRound },
  { id: "snippets", icon: Code },
  { id: "tunnels", icon: ArrowLeftRight },
  { id: "team", icon: Users },
];

export function Sidebar({ homeActive }: { homeActive: boolean }) {
  const { ws, vault, section, navigate, openPalette, resolveConflicts } = useStore();
  const t = useT().app;
  const conflictHost = new Set(vault.conflicts.map((c) => c.record)).size === 1 && vault.conflicts[0]!.coll === "host" ? ws.hosts.find((h) => h.id === vault.conflicts[0]!.record) : null;

  const counts: Partial<Record<Section, number>> = {
    hosts: ws.hosts.filter((h) => h.vaultId === vault.id).length,
    keys: ws.keys.filter((k) => k.vaultId === vault.id).length,
    snippets: ws.snippets.filter((s) => s.vaultId === vault.id).length,
    tunnels: ws.tunnels.filter((t) => t.vaultId === vault.id).length,
    team: vault.kind === "team" ? ws.members.filter((m) => m.vaultId === vault.id).length : undefined,
  };

  return (
    <aside className="sidebar">
      <VaultSwitcher />

      {vault.conflicts.length > 0 && (
        <button className="sidebar-alert" onClick={() => resolveConflicts()}>
          <TriangleAlert size={15} />
          <span className="grow ellipsis">
            {conflictHost ? t.conflictWith(conflictHost.label) : t.conflicts(vault.conflicts.length)}
          </span>
          <span className="sidebar-alert-action">{t.resolve}</span>
        </button>
      )}

      <button className="quick" onClick={openPalette}>
        <Search size={14} />
        <span>{t.quickConnect}</span>
        <Kbd>Ctrl K</Kbd>
      </button>

      <nav className="nav">
        {nav.map(({ id, icon: Icon }) => (
          <button key={id} className={`nav-item ${section === id && homeActive ? "active" : ""}`} onClick={() => navigate(id)}>
            <Icon size={16} />
            <span>{id === "team" && vault.kind === "personal" ? t.section.teams : t.section[id]}</span>
            {!!counts[id] && <span className="nav-count">{counts[id]}</span>}
          </button>
        ))}
      </nav>

      <div className="grow" />

      <nav className="nav">
        <button className={`nav-item ${section === "settings" && homeActive ? "active" : ""}`} onClick={() => navigate("settings")}>
          <Settings size={16} />
          <span>{t.section.settings}</span>
        </button>
      </nav>
    </aside>
  );
}

function VaultSwitcher() {
  const { ws, vault, setVault, navigate, sync } = useStore();
  const all = useT();
  const t = all.app;
  const [open, setOpen] = useState(false);
  const [dialog, setDialog] = useState<"create" | "join" | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", key);
    };
  }, [open]);

  const pick = (v: Vault) => {
    setVault(v.id);
    setOpen(false);
  };

  return (
    <div className="vault-switcher" ref={ref}>
      <button className={`vault-current ${open ? "open" : ""}`} onClick={() => setOpen((o) => !o)}>
        <VaultAvatar vault={vault} size={26} />
        <span className="vault-names">
          <span className="vault-name">{vault.name}</span>
          {vault.kind === "team" && (
            <span className={`vault-sync ${vault.removed || vault.remoteError ? "conflict" : vault.pending ? "syncing" : vault.syncStatus}`}>
              <span className="dot" />
              {vault.syncStatus === "syncing" || vault.syncStatus === "offline" ? all.common.syncStatus[vault.syncStatus] : vault.removed ? t.removedFromTeam : vault.joined === false ? t.waitingToBeAdded : vault.remoteError ? t.repoUnreachable : vault.pending ? t.notSent : formatAgo(vault.lastSync)}
            </span>
          )}
          {vault.kind === "personal" && sync?.enabled && (
            <span className={`vault-sync ${sync.error ? "conflict" : sync.pending ? "syncing" : "synced"}`}>
              <span className="dot" />
              {sync.error ? syncErrorText(sync) : sync.pending ? t.notSent : formatAgo(sync.syncedAt ?? undefined)}
            </span>
          )}
        </span>
        <ChevronsUpDown size={14} className="muted" />
      </button>

      {open && (
        <div className="popover vault-menu">
          {ws.vaults.map((v) => (
            <button key={v.id} className="menu-row" onClick={() => pick(v)}>
              <VaultAvatar vault={v} size={20} />
              <span className="grow">{v.name}</span>
              {v.conflicts.length > 0 && v.id !== vault.id && <span className="chip warn">{t.conflict}</span>}
              {v.id === vault.id && <Check size={14} className="accent" />}
            </button>
          ))}
          <div className="menu-sep" />
          <button className="menu-row" onClick={() => { setOpen(false); setDialog("create"); }}>
            <Plus size={15} className="muted" /> {t.createTeam}
          </button>
          <button className="menu-row" onClick={() => { setOpen(false); setDialog("join"); }}>
            <LogIn size={15} className="muted" /> {t.joinTeam}
          </button>
        </div>
      )}

      {dialog === "create" && <CreateTeamDialog onClose={() => setDialog(null)} onDone={() => navigate("team")} />}
      {dialog === "join" && <JoinTeamDialog onClose={() => setDialog(null)} onDone={() => navigate("team")} />}
    </div>
  );
}
