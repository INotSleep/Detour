import { useCallback, useEffect, useMemo, useState } from "react";
import { ArrowLeftRight, Code, KeyRound, Plus, Server, Settings as SettingsIcon, Users, X } from "lucide-react";
import { api, type CoreInfo } from "./lib/api";
import { StoreContext, uid, type Section, type Store } from "./lib/store";
import type { Settings, Workspace } from "./lib/types";
import type { SessionStatus } from "./lib/fakeShell";
import { Sidebar } from "./components/Sidebar";
import { HostsView } from "./components/HostsView";
import { TerminalView } from "./components/TerminalView";
import { TeamsView } from "./components/TeamsView";
import { SettingsView, SnippetsView } from "./components/OtherViews";
import { KeysView } from "./components/KeysView";
import { TunnelsView } from "./components/TunnelsView";
import { CommandPalette } from "./components/CommandPalette";
import { ConflictDialog } from "./components/ConflictDialog";
import { IconButton, useToaster } from "./components/ui";

interface Session {
  id: string;
  hostId: string;
  title: string;
  status: SessionStatus;
  command?: string;
}

const sectionMeta: Record<Section, { title: string; icon: typeof Server }> = {
  hosts: { title: "Хосты", icon: Server },
  keys: { title: "Ключи", icon: KeyRound },
  snippets: { title: "Сниппеты", icon: Code },
  tunnels: { title: "Туннели", icon: ArrowLeftRight },
  team: { title: "Команда", icon: Users },
  settings: { title: "Настройки", icon: SettingsIcon },
};

const defaultSettings: Settings = { theme: "dark", fontSize: 13, scrollback: 5000, autoLockMinutes: 15 };

export default function App() {
  const [ws, setWs] = useState<Workspace | null>(null);
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null);
  const [vaultId, setVaultId] = useState("acme");
  const [section, setSection] = useState<Section>("hosts");
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeId, setActiveId] = useState("home");
  const [palette, setPalette] = useState(false);
  const [conflictVaultId, setConflictVaultId] = useState<string | null>(null);
  const [settings, setSettingsState] = useState<Settings>(defaultSettings);
  const toaster = useToaster();

  useEffect(() => {
    api.loadWorkspace().then(setWs);
    api.coreInfo().then(setCoreInfo).catch(() => setCoreInfo(null));
  }, []);

  useEffect(() => {
    const apply = () => {
      const dark = settings.theme === "dark" || (settings.theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
    };
    apply();
    const mq = matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [settings.theme]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const connect = useCallback(
    (hostId: string, command?: string) => {
      const host = ws?.hosts.find((h) => h.id === hostId);
      if (!host) return;
      const id = uid("s");
      setSessions((list) => {
        const same = list.filter((s) => s.hostId === hostId).length;
        return [...list, { id, hostId, title: same ? `${host.label} (${same + 1})` : host.label, status: "connecting", command }];
      });
      setActiveId(id);
    },
    [ws],
  );

  const closeSession = (id: string) => {
    const idx = sessions.findIndex((s) => s.id === id);
    const next = sessions.filter((s) => s.id !== id);
    setSessions(next);
    if (activeId === id) setActiveId(next[Math.min(idx, next.length - 1)]?.id ?? "home");
  };

  const setStatus = (id: string, status: SessionStatus) => setSessions((list) => list.map((s) => (s.id === id ? { ...s, status } : s)));

  const store = useMemo<Store | null>(() => {
    if (!ws) return null;
    return {
      ws,
      update: (fn) => setWs((w) => (w ? fn(w) : w)),
      vault: ws.vaults.find((v) => v.id === vaultId)!,
      setVault: (id) => {
        setVaultId(id);
        setActiveId("home");
      },
      section,
      navigate: (s) => {
        setSection(s);
        setActiveId("home");
      },
      settings,
      setSettings: (patch) => setSettingsState((s) => ({ ...s, ...patch })),
      coreInfo,
      toast: toaster.push,
      connect,
      openPalette: () => setPalette(true),
      resolveConflicts: (id) => setConflictVaultId(id ?? vaultId),
    };
  }, [ws, vaultId, section, settings, coreInfo, connect, toaster.push]);

  if (!store) return null;

  const conflictVault = ws!.vaults.find((v) => v.id === conflictVaultId);
  const Home = section === "team" && store.vault.kind === "personal" ? { ...sectionMeta.team, title: "Команды" } : sectionMeta[section];

  return (
    <StoreContext.Provider value={store}>
      <div className="app">
        <Sidebar homeActive={activeId === "home"} />

        <main className="main">
          {sessions.length > 0 && (
            <div className="tabbar">
              <button className={`tab ${activeId === "home" ? "active" : ""}`} onClick={() => setActiveId("home")}>
                <Home.icon size={14} />
                <span className="tab-label">{Home.title}</span>
              </button>
              {sessions.map((s) => (
                <div
                  key={s.id}
                  role="tab"
                  tabIndex={0}
                  aria-selected={activeId === s.id}
                  className={`tab ${activeId === s.id ? "active" : ""}`}
                  onClick={() => setActiveId(s.id)}
                  onAuxClick={(e) => e.button === 1 && closeSession(s.id)}
                >
                  <span className={`session-dot ${s.status}`} />
                  <span className="tab-label">{s.title}</span>
                  <button
                    className="tab-close"
                    aria-label="Закрыть"
                    onClick={(e) => {
                      e.stopPropagation();
                      closeSession(s.id);
                    }}
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
              <IconButton icon={Plus} label="Новое подключение · Ctrl K" className="tab-new" onClick={() => setPalette(true)} />
            </div>
          )}

          <div className="content">
            <div className={`view ${activeId === "home" ? "" : "hidden"}`}>
              {section === "hosts" && <HostsView key={vaultId} />}
              {section === "keys" && <KeysView />}
              {section === "snippets" && <SnippetsView />}
              {section === "tunnels" && <TunnelsView />}
              {section === "team" && <TeamsView />}
              {section === "settings" && <SettingsView />}
            </div>
            {sessions.map((s) => {
              const host = ws!.hosts.find((h) => h.id === s.hostId);
              if (!host) return null;
              return (
                <div key={s.id} className={`view ${activeId === s.id ? "" : "hidden"}`}>
                  <TerminalView host={host} active={activeId === s.id} initialCommand={s.command} onStatus={(st) => setStatus(s.id, st)} onExit={() => closeSession(s.id)} />
                </div>
              );
            })}
          </div>
        </main>
      </div>

      {palette && <CommandPalette onClose={() => setPalette(false)} />}
      {conflictVault && conflictVault.conflicts.length > 0 && <ConflictDialog vault={conflictVault} onClose={() => setConflictVaultId(null)} />}
      {toaster.view}
    </StoreContext.Provider>
  );
}
