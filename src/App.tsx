import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftRight, Code, KeyRound, Plus, Server, Settings as SettingsIcon, Users, X } from "lucide-react";
import { api, errorText, fromView, type CoreInfo, type TeamView, type VaultView } from "./lib/api";
import { applyTeamEntries, applyTeamView, dataOf, writable } from "./lib/teams";
import { mockWorkspace } from "./lib/mockData";
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
import { LockScreen } from "./components/LockScreen";

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

type Phase = "loading" | "setup" | "locked" | "ready";

const emptyWorkspace = (): Workspace => ({
  vaults: [{ id: "personal", name: "Личное", kind: "personal", sync: { type: "local" }, syncStatus: "synced", conflicts: [] }],
  hosts: [],
  members: [],
  keys: [],
  snippets: [],
  tunnels: [],
  activity: [],
  access: [],
});

/** The real personal vault and teams from detour-core; demo data in a plain browser. */
async function loadWorkspace(view: VaultView | null): Promise<Workspace> {
  if (!api.real) return structuredClone(mockWorkspace);
  const personal = fromView(view ?? (await api.vaultGet()));
  const ws = { ...emptyWorkspace(), ...personal };
  return applyTeamEntries(ws, await api.teamsList());
}

export default function App() {
  const [phase, setPhase] = useState<Phase>("loading");
  // What each vault looked like when last saved to / read from detour-core.
  const lastSaved = useRef(new Map<string, string>());
  const [ws, setWs] = useState<Workspace | null>(null);
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null);
  const [vaultId, setVaultId] = useState(api.real ? "personal" : "acme");
  const [section, setSection] = useState<Section>("hosts");
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeId, setActiveId] = useState("home");
  const [palette, setPalette] = useState(false);
  const [conflictVaultId, setConflictVaultId] = useState<string | null>(null);
  const [settings, setSettingsState] = useState<Settings>(defaultSettings);
  const toaster = useToaster();

  useEffect(() => {
    api.coreInfo().then(setCoreInfo).catch(() => setCoreInfo(null));
    api.vaultStatus().then(async (st) => {
      if (!st.initialized) return setPhase("setup");
      if (!st.unlocked) return setPhase("locked");
      setWs(await loadWorkspace(null));
      setPhase("ready");
    });
  }, []);

  const unlocked = async (view: VaultView | null) => {
    lastSaved.current.clear();
    const fresh = await loadWorkspace(view);
    // Keep tunnel switches and other in-memory state of a workspace that was only locked.
    setWs((prev) => (prev && api.real ? { ...fresh, tunnels: fresh.tunnels.map((t) => prev.tunnels.find((p) => p.id === t.id) ?? t) } : prev ?? fresh));
    setPhase("ready");
  };

  const lock = useCallback(async () => {
    await api.vaultLock();
    setPhase("locked");
  }, []);

  // The latest workspace, for decisions made outside React's state updaters.
  // Updaters must stay pure: React may run them twice, and a side effect there
  // once made a client save an empty host list and delete a team's hosts.
  const wsRef = useRef<Workspace | null>(null);
  wsRef.current = ws;

  /** Puts a team's state from the core into the workspace, unless local edits are still on their way. */
  const applyView = useCallback((view: TeamView) => {
    const base = wsRef.current;
    if (!base) return;
    const saved = lastSaved.current.get(view.id);
    const pending = saved !== undefined && saved !== JSON.stringify(dataOf(base, view.id));
    const next = applyTeamView(base, view, !pending);
    if (!pending) lastSaved.current.set(view.id, JSON.stringify(dataOf(next, view.id)));
    wsRef.current = next;
    // Edited in the meantime: keep those edits, take only members, conflicts and activity.
    setWs((cur) => (cur === base ? next : cur && applyTeamView(cur, view, false)));
  }, []);

  // Persist edits of every writable vault to detour-core.
  useEffect(() => {
    if (!api.real || !ws || phase !== "ready") return;
    for (const v of ws.vaults) {
      const data = dataOf(ws, v.id);
      const json = JSON.stringify(data);
      const prev = lastSaved.current.get(v.id);
      if (prev === undefined || !writable(v)) {
        lastSaved.current.set(v.id, json);
        continue;
      }
      if (json === prev) continue;
      lastSaved.current.set(v.id, json);
      const fail = (e: unknown) => toaster.push({ text: `Не удалось сохранить: ${errorText(e)}`, tone: "error" });
      if (v.kind === "personal") {
        api
          .vaultPut(data)
          .then(() => {
            // A typed password is now in the vault: drop it from UI memory without another save.
            const base = wsRef.current;
            if (!base || !data.hosts.some((h) => h.newPassword)) return;
            const next = { ...base, hosts: base.hosts.map(({ newPassword: _p, ...h }) => h) };
            lastSaved.current.set(v.id, JSON.stringify(dataOf(next, v.id)));
            wsRef.current = next;
            setWs((cur) => (cur === base ? next : cur));
          })
          .catch(fail);
      } else {
        api.teamPut(v.id, data).then(applyView).catch(fail);
      }
    }
  }, [ws, phase, toaster.push, applyView]);

  // Pick up teammates' changes in the background.
  const teamIds = ws?.vaults.filter((v) => v.kind === "team").map((v) => v.id).join(",") ?? "";
  useEffect(() => {
    if (!api.real || phase !== "ready" || !teamIds) return;
    const timer = setInterval(() => {
      for (const id of teamIds.split(",")) api.teamSync(id).then(applyView).catch(() => {});
    }, 20_000);
    return () => clearInterval(timer);
  }, [phase, teamIds, applyView]);

  // Auto-lock after inactivity.
  useEffect(() => {
    if (!api.real || phase !== "ready" || !settings.autoLockMinutes) return;
    let last = Date.now();
    const touch = () => (last = Date.now());
    const events = ["keydown", "mousedown", "wheel"] as const;
    events.forEach((e) => window.addEventListener(e, touch, true));
    const timer = setInterval(() => Date.now() - last > settings.autoLockMinutes * 60_000 && lock(), 15_000);
    return () => {
      events.forEach((e) => window.removeEventListener(e, touch, true));
      clearInterval(timer);
    };
  }, [phase, settings.autoLockMinutes, lock]);

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
      vault: ws.vaults.find((v) => v.id === vaultId) ?? ws.vaults[0]!,
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
      lock,
      applyTeamView: applyView,
    };
  }, [ws, vaultId, section, settings, coreInfo, connect, toaster.push, lock, applyView]);

  if (phase === "loading") return null;
  if (!store) return <LockScreen mode={phase === "setup" ? "setup" : "unlock"} onUnlocked={(v) => void unlocked(v)} />;

  const conflictVault = ws!.vaults.find((v) => v.id === conflictVaultId);
  const Home = section === "team" && store.vault.kind === "personal" ? { ...sectionMeta.team, title: "Команды" } : sectionMeta[section];

  return (
    <StoreContext.Provider value={store}>
      <div className="app" inert={phase !== "ready"}>
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
      {phase === "locked" && <LockScreen mode="unlock" onUnlocked={(v) => void unlocked(v)} />}
    </StoreContext.Provider>
  );
}
