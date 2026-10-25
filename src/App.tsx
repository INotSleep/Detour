import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftRight, Code, FolderOpen, FolderPlus, KeyRound, Plus, Server, Settings as SettingsIcon, Users, X } from "lucide-react";
import { LangContext, lang as currentLang, resolveLang, setLang, tr, useT } from "./i18n";
import { api, errorText, fromView, type CoreInfo, type SyncInfo, type TeamView, type VaultView } from "./lib/api";
import { applyTeamEntries, applyTeamView, dataOf, removeVault, writable } from "./lib/teams";
import { mockWorkspace } from "./lib/mockData";
import { StoreContext, shortcutKey, uid, type Section, type Store, type UpdateState } from "./lib/store";
import { checkForUpdate, installUpdate as installFound, updateErrorText } from "./lib/updates";
import type { Settings, Workspace } from "./lib/types";
import type { SessionStatus } from "./lib/fakeShell";
import { Sidebar } from "./components/Sidebar";
import { HostsView } from "./components/HostsView";
import { TerminalTab } from "./components/TerminalTab";
import { TeamsView } from "./components/TeamsView";
import { SettingsView, SnippetsView } from "./components/OtherViews";
import { KeysView } from "./components/KeysView";
import { TunnelsView } from "./components/TunnelsView";
import { FilesView } from "./components/FilesView";
import { CommandPalette } from "./components/CommandPalette";
import { ConflictDialog } from "./components/ConflictDialog";
import { IconButton, useToaster } from "./components/ui";
import { LockScreen } from "./components/LockScreen";

interface Session {
  id: string;
  /** Null for an SFTP tab whose host is not chosen yet. */
  hostId: string | null;
  /** A terminal or a file browser. */
  kind: "terminal" | "files";
  title: string;
  status: SessionStatus;
  command?: string;
}

const sectionIcon: Record<Section, typeof Server> = {
  hosts: Server,
  keys: KeyRound,
  snippets: Code,
  tunnels: ArrowLeftRight,
  team: Users,
  settings: SettingsIcon,
};

const defaultSettings: Settings = { theme: "dark", language: "system", fontSize: 13, scrollback: 5000, autoLockMinutes: 15, clipboard: "ssh", autoUpdate: true, editor: null };
const SETTINGS_KEY = "detour.settings";

/** Settings of this device, kept in the webview's storage. */
function loadSettings(): Settings {
  try {
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}");
    // An earlier version chose only what a right click does.
    if (!saved.clipboard && saved.rightClick === "menu") saved.clipboard = "termius";
    delete saved.rightClick;
    return { ...defaultSettings, ...saved };
  } catch {
    return defaultSettings;
  }
}

type Phase = "loading" | "setup" | "locked" | "ready";

const emptyWorkspace = (): Workspace => ({
  vaults: [{ id: "personal", name: tr().app.personal, kind: "personal", sync: { type: "local" }, syncStatus: "synced", conflicts: [] }],
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
  if (!api.real) return mockWorkspace();
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
  const [settings, setSettingsState] = useState<Settings>(loadSettings);
  useEffect(() => {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      /* private mode: settings last for this run */
    }
  }, [settings]);
  const [sync, setSync] = useState<SyncInfo | null>(null);
  const [updateState, setUpdateState] = useState<UpdateState>({ kind: "idle" });
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
  // Pending re-reads of Git teams after an edit, and the latest applyView for them.
  const recheck = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const applyViewRef = useRef<(view: TeamView) => void>(() => {});
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
    // An edit of a Git team is pushed in the background; look again once it had time to go out.
    if (view.remote?.pending && !view.remote.error) {
      clearTimeout(recheck.current.get(view.id));
      recheck.current.set(view.id, setTimeout(() => api.teamSync(view.id).then(applyViewRef.current).catch(() => {}), 1500));
    }
  }, []);
  applyViewRef.current = applyView;

  /** Takes the personal vault from the core again, unless local edits are still on their way. */
  const refreshPersonal = useCallback(async () => {
    const view = await api.vaultGet();
    const base = wsRef.current;
    if (!base) return;
    const saved = lastSaved.current.get("personal");
    if (saved !== undefined && saved !== JSON.stringify(dataOf(base, "personal"))) return;
    const data = fromView(view);
    const others = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId !== "personal");
    // A tunnel's switch and connections belong to this session, not to the vault.
    const running = new Map(base.tunnels.map((t) => [t.id, t]));
    const next: Workspace = {
      ...base,
      hosts: [...data.hosts, ...others(base.hosts)],
      keys: [...data.keys, ...others(base.keys)],
      snippets: [...data.snippets, ...others(base.snippets)],
      tunnels: [...data.tunnels.map((t) => ({ ...t, state: running.get(t.id)?.state ?? t.state, connections: running.get(t.id)?.connections, error: running.get(t.id)?.error })), ...others(base.tunnels)],
    };
    lastSaved.current.set("personal", JSON.stringify(dataOf(next, "personal")));
    wsRef.current = next;
    setWs((cur) => (cur === base ? next : cur));
  }, []);

  /* ---------- new versions ---------- */

  const installUpdate = useCallback(async () => {
    const st = updateStateRef.current;
    if (st.kind !== "available") return;
    setUpdateState({ kind: "installing", info: st.info, share: null });
    try {
      // The app restarts into the new version when this finishes.
      await installFound((share) => setUpdateState({ kind: "installing", info: st.info, share }));
    } catch (e) {
      setUpdateState({ kind: "error", text: updateErrorText(e) });
    }
  }, []);
  const updateStateRef = useRef(updateState);
  updateStateRef.current = updateState;

  const checkUpdate = useCallback(async (quiet = false) => {
    if (!api.real) return setUpdateState({ kind: "latest" });
    setUpdateState({ kind: "checking" });
    try {
      const info = await checkForUpdate();
      setUpdateState(info ? { kind: "available", info } : { kind: "latest" });
      if (info && quiet) toaster.push({ text: tr().app.updateAvailable(info.version), duration: 15000, action: { label: tr().app.update, run: () => void installUpdate() } });
    } catch (e) {
      // A check at start that fails says nothing; one asked for says why.
      setUpdateState(quiet ? { kind: "idle" } : { kind: "error", text: updateErrorText(e) });
    }
  }, [toaster.push, installUpdate]);

  // Look for a new version a little after start.
  useEffect(() => {
    if (!api.real || phase !== "ready" || !settings.autoUpdate) return;
    const timer = setTimeout(() => void checkUpdate(true), 8000);
    return () => clearTimeout(timer);
    // Once per unlock.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  /** Takes the list of teams from the core again: teams joined or left on another device. */
  const refreshTeams = useCallback(async () => {
    const entries = await api.teamsList();
    setWs((w) => {
      if (!w) return w;
      const left = w.vaults.filter((v) => v.kind === "team" && !entries.some((e) => e.id === v.id));
      return applyTeamEntries(left.reduce((acc, v) => removeVault(acc, v.id), w), entries);
    });
  }, []);

  // The personal vault's sync: its state, and a sync every 20 seconds while it is on.
  useEffect(() => {
    if (phase === "ready") api.syncStatus().then(setSync).catch(() => {});
  }, [phase]);
  const syncOn = sync?.enabled ?? false;
  useEffect(() => {
    if (!api.real || phase !== "ready" || !syncOn) return;
    const run = () =>
      api
        .syncNow()
        .then((r) => {
          setSync(r.status);
          if (r.changed) {
            void refreshPersonal();
            void refreshTeams();
          }
        })
        .catch(() => {});
    void run();
    const timer = setInterval(run, 20_000);
    return () => clearInterval(timer);
  }, [phase, syncOn, refreshPersonal, refreshTeams]);
  // After an edit the core merges and sends it; look at the outcome once it had time to go out.
  const syncCheck = useRef<ReturnType<typeof setTimeout>>(undefined);
  const afterPersonalEdit = useCallback(() => {
    clearTimeout(syncCheck.current);
    syncCheck.current = setTimeout(() => api.syncStatus().then(setSync).catch(() => {}), 1500);
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
      const fail = (e: unknown) => toaster.push({ text: tr().common.saveFailed(errorText(e)), tone: "error" });
      if (v.kind === "personal") {
        api
          .vaultPut(data)
          .then(() => {
            if (syncOn) afterPersonalEdit();
            // A typed password is now in the vault: drop it from UI memory without another save.
            const base = wsRef.current;
            if (!base || !data.hosts.some((h) => h.newPassword || h.newProxyPassword)) return;
            const next = { ...base, hosts: base.hosts.map(({ newPassword: _p, newProxyPassword: _q, ...h }) => h) };
            lastSaved.current.set(v.id, JSON.stringify(dataOf(next, v.id)));
            wsRef.current = next;
            setWs((cur) => (cur === base ? next : cur));
          })
          .catch(fail);
      } else {
        api.teamPut(v.id, data).then(applyView).catch(fail);
      }
    }
  }, [ws, phase, toaster.push, applyView, syncOn, afterPersonalEdit]);

  // Tunnels run in the core: keep switches, connection counts and failures in step with it.
  const tunnelsActive = ws?.tunnels.some((t) => t.state === "on" || t.state === "starting") ?? false;
  useEffect(() => {
    if (!api.real || phase !== "ready") return;
    const poll = () =>
      api
        .tunnelsStatus()
        .then((list) => {
          const byKey = new Map(list.map((s) => [s.key, s]));
          setWs((w) =>
            w && {
              ...w,
              tunnels: w.tunnels.map((t) => {
                const s = byKey.get(`${t.vaultId}/${t.id}`);
                if (t.state === "starting") return t;
                if (!s) return t.state === "on" ? { ...t, state: "off" as const, connections: 0 } : t;
                if (s.state === "error") return { ...t, state: "error" as const, connections: 0, error: s.error ? errorText(s.error) : undefined };
                return t.state === "on" && t.connections === s.connections ? t : { ...t, state: "on" as const, connections: s.connections, error: undefined };
              }),
            },
          );
        })
        .catch(() => {});
    void poll();
    if (!tunnelsActive) return;
    const timer = setInterval(poll, 1500);
    return () => clearInterval(timer);
  }, [phase, tunnelsActive]);

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

  // Text follows the language right away: set it before anything below renders.
  const lang = resolveLang(settings.language);
  if (currentLang() !== lang) setLang(lang);
  useEffect(() => {
    setWs((cur) => cur && { ...cur, vaults: cur.vaults.map((v) => (v.kind === "personal" ? { ...v, name: tr().app.personal } : v)) });
  }, [lang]);

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

  // The webview's own menu (Back, Reload, Print) means nothing here; text fields keep theirs.
  useEffect(() => {
    const block = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest?.("input, textarea, [contenteditable=\"true\"]")) e.preventDefault();
    };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && shortcutKey(e) === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const openTab = useCallback(
    (hostId: string, kind: Session["kind"], command?: string) => {
      const host = ws?.hosts.find((h) => h.id === hostId);
      if (!host) return;
      const id = uid("s");
      setSessions((list) => {
        const same = list.filter((s) => s.hostId === hostId && s.kind === kind).length;
        return [...list, { id, hostId, kind, title: same ? `${host.label} (${same + 1})` : host.label, status: "connecting", command }];
      });
      setActiveId(id);
    },
    [ws],
  );
  const connect = useCallback((hostId: string, command?: string) => openTab(hostId, "terminal", command), [openTab]);
  const openFiles = useCallback((hostId: string) => openTab(hostId, "files"), [openTab]);
  /** An SFTP tab that first asks which host to open. */
  const newFiles = () => {
    const id = uid("s");
    setSessions((list) => [...list, { id, hostId: null, kind: "files", title: tr().app.sftp, status: "closed" }]);
    setActiveId(id);
  };
  const pickHost = (id: string, hostId: string) => {
    const host = ws?.hosts.find((h) => h.id === hostId);
    if (!host) return;
    setSessions((list) => {
      const same = list.filter((s) => s.hostId === hostId && s.kind === "files").length;
      return list.map((s) => (s.id === id ? { ...s, hostId, title: same ? `${host.label} (${same + 1})` : host.label, status: "connecting" } : s));
    });
  };

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
      openFiles,
      openPalette: () => setPalette(true),
      resolveConflicts: (id) => setConflictVaultId(id ?? vaultId),
      lock,
      applyTeamView: applyView,
      sync,
      setSync,
      refreshPersonal,
      update_: updateState,
      checkUpdate: () => checkUpdate(false),
      installUpdate,
    };
  }, [ws, vaultId, section, settings, coreInfo, connect, openFiles, toaster.push, lock, applyView, sync, refreshPersonal, updateState, checkUpdate, installUpdate]);

  if (phase === "loading") return null;
  if (!store)
    return (
      <LangContext.Provider value={lang}>
        <LockScreen mode={phase === "setup" ? "setup" : "unlock"} onUnlocked={(v) => void unlocked(v)} />
      </LangContext.Provider>
    );

  const conflictVault = ws!.vaults.find((v) => v.id === conflictVaultId);
  return (
    <LangContext.Provider value={lang}>
      <StoreContext.Provider value={store}>
        <div className="app" inert={phase !== "ready"}>
          <Sidebar homeActive={activeId === "home"} />

          <main className="main">
            <div className="tabbar">
                <HomeTab section={section} personal={store.vault.kind === "personal"} active={activeId === "home"} onClick={() => setActiveId("home")} />
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
                    {s.kind === "files" && <FolderOpen size={13} className="tab-kind" />}
                    <span className="tab-label">{s.title}</span>
                    <button
                      className="tab-close"
                      aria-label={tr().common.close}
                      onClick={(e) => {
                        e.stopPropagation();
                        closeSession(s.id);
                      }}
                    >
                      <X size={12} />
                    </button>
                  </div>
                ))}
                <IconButton icon={Plus} label={tr().app.newConnection} className="tab-new" onClick={() => setPalette(true)} />
                <IconButton icon={FolderPlus} label={tr().app.newSftp} className="tab-new" onClick={newFiles} />
              </div>

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
                const host = ws!.hosts.find((h) => h.id === s.hostId) ?? null;
                if (!host && !(s.kind === "files" && s.hostId === null)) return null;
                return (
                  <div key={s.id} className={`view ${activeId === s.id ? "" : "hidden"}`}>
                    {s.kind === "files" || !host ? (
                      <FilesView host={host} active={activeId === s.id} onStatus={(st) => setStatus(s.id, st)} onClose={() => closeSession(s.id)} onPickHost={(hostId) => pickHost(s.id, hostId)} />
                    ) : (
                      <TerminalTab host={host} active={activeId === s.id} initialCommand={s.command} onStatus={(st) => setStatus(s.id, st)} onExit={() => closeSession(s.id)} />
                    )}
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
    </LangContext.Provider>
  );
}

/** The first tab: the section open in the sidebar. */
function HomeTab({ section, personal, active, onClick }: { section: Section; personal: boolean; active: boolean; onClick: () => void }) {
  const t = useT().app.section;
  const Icon = sectionIcon[section];
  return (
    <button className={`tab ${active ? "active" : ""}`} onClick={onClick}>
      <Icon size={14} />
      <span className="tab-label">{section === "team" && personal ? t.teams : t[section]}</span>
    </button>
  );
}
