import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftRight, Bot, Code, Columns2, FolderOpen, FolderPlus, KeyRound, Plus, Rows2, Server, Settings as SettingsIcon, Shield, Users, X } from "lucide-react";
import { LangContext, lang as currentLang, resolveLang, setLang, tr, useT } from "./i18n";
import { api, errorText, fromView, isCoreError, type AgentRequest, type CoreInfo, type SyncInfo, type TeamView, type VaultView } from "./lib/api";
import { shownSessions } from "./lib/realSession";
import { applyTeamEntries, applyTeamView, dataOf, removeVault, writable } from "./lib/teams";
import { mockWorkspace } from "./lib/mockData";
import { StoreContext, uid, type Section, type Store, type UpdateState } from "./lib/store";
import { hit, keysOf, recording, setOverrides } from "./lib/shortcuts";
import { MAX_PANES, carried, insert, isPane, neighbour, panes as panesOf, remove as removePane, type Layout, type Side, type SplitDir } from "./lib/panes";
import { checkForUpdate, installUpdate as installFound, updateErrorText } from "./lib/updates";
import type { Settings, Workspace } from "./lib/types";
import { SAME_AS_APP, SYSTEM, appColors, builtinThemes, findTheme, isLight, terminalColors, type Theme } from "./lib/themes";
import type { SessionStatus } from "./lib/fakeShell";
import { Sidebar } from "./components/Sidebar";
import { HostsView } from "./components/HostsView";
import { TerminalTab } from "./components/TerminalTab";
import { TeamsView } from "./components/TeamsView";
import { SettingsView, SnippetsView } from "./components/OtherViews";
import { KeysView } from "./components/KeysView";
import { TunnelsView } from "./components/TunnelsView";
import { ProxiesView, savedAfter } from "./components/ProxiesView";
import { FilesView } from "./components/FilesView";
import { CommandPalette } from "./components/CommandPalette";
import { ConflictDialog } from "./components/ConflictDialog";
import { Button, ContextMenu, IconButton, Modal, TipLayer, modalOpen, useToaster, type MenuItem } from "./components/ui";
import { LockScreen } from "./components/LockScreen";
import { AgentRequestDialog } from "./components/Agents";

interface Session {
  id: string;
  /** Null for an SFTP tab whose host is not chosen yet. */
  hostId: string | null;
  /** A terminal or a file browser. */
  kind: "terminal" | "files";
  title: string;
  status: SessionStatus;
  command?: string;
  /** A session the core already holds (opened before the window was, or by an agent). */
  attach?: string;
  /** The AI agent whose session it is. */
  agent?: string;
  /** A terminal tab's panes, and the one with focus. */
  layout?: Layout;
  focused?: string;
}

const onePane = (hostId: string, extra: { command?: string; attach?: string } = {}) => {
  const id = uid("p");
  return { layout: { pane: { id, hostId, ...extra } } as Layout, focused: id };
};

const sectionIcon: Record<Section, typeof Server> = {
  hosts: Server,
  keys: KeyRound,
  snippets: Code,
  tunnels: ArrowLeftRight,
  proxies: Shield,
  team: Users,
  settings: SettingsIcon,
};

const defaultSettings: Settings = { theme: "detour-dark", language: "system", fontSize: 13, scrollback: 5000, autoLockMinutes: 15, clipboard: "ssh", highlight: true, autoUpdate: true, editor: null, sidebarCollapsed: false, terminalTheme: SAME_AS_APP, customThemes: [], shortcuts: {} };
const SETTINGS_KEY = "detour.settings";
const VAULT_KEY = "detour.vault";

/** Settings of this device, kept in the webview's storage. */
function loadSettings(): Settings {
  try {
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}");
    // An earlier version chose only what a right click does.
    if (!saved.clipboard && saved.rightClick === "menu") saved.clipboard = "termius";
    delete saved.rightClick;
    // Earlier versions had a dark and a light look, and terminals one palette.
    if (saved.theme === "dark" || saved.theme === "light") saved.theme = `detour-${saved.theme}`;
    if (saved.terminalTheme === "detour") saved.terminalTheme = SAME_AS_APP;
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
  proxies: [],
  groups: [],
  layouts: {},
  groupPrefs: [],
  activity: [],
  access: [],
});

/** The real personal vault and teams from detour-core; demo data in a plain browser. */
async function loadWorkspace(view: VaultView | null): Promise<Workspace> {
  if (!api.real) return mockWorkspace();
  const personal = fromView(view ?? (await api.vaultGet()));
  const ws = { ...emptyWorkspace(), ...personal, groupPrefs: await api.groupPrefs().catch(() => []) };
  return applyTeamEntries(ws, await api.teamsList());
}

export default function App() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [startError, setStartError] = useState<unknown>(null);
  // What each vault looked like when last saved to / read from detour-core.
  const lastSaved = useRef(new Map<string, string>());
  // What each vault looks like as the core last sent it. Saving compares with it: a workspace
  // showing exactly that is the core's state, not an edit. Taking a sent state must never mark it
  // saved before the window shows it: an older picture still on screen would then look like an
  // edit back to old values, and saving it once undid other members' changes and deleted a host.
  const received = useRef(new Map<string, string>());
  const [ws, setWs] = useState<Workspace | null>(null);
  const [coreInfo, setCoreInfo] = useState<CoreInfo | null>(null);
  // The vault shown when the window was last closed; the personal one if that team is gone.
  const [vaultId, setVaultId] = useState(() => {
    try {
      return localStorage.getItem(VAULT_KEY) ?? (api.real ? "personal" : "acme");
    } catch {
      return api.real ? "personal" : "acme";
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(VAULT_KEY, vaultId);
    } catch {
      /* private mode: chosen for this run only */
    }
  }, [vaultId]);
  const [section, setSection] = useState<Section>("hosts");
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeId, setActiveId] = useState("home");
  const [palette, setPalette] = useState(false);
  const [conflictVaultId, setConflictVaultId] = useState<string | null>(null);
  /** A terminal waiting for "Allow" to use Detour from the command line. */
  const [trustAsk, setTrustAsk] = useState<{ id: number; terminal: { name: string; pid: number; host: string | null } | null } | null>(null);
  /** What AI agents wait on, oldest first. */
  const [agentAsks, setAgentAsks] = useState<AgentRequest[]>([]);
  const [settings, setSettingsState] = useState<Settings>(loadSettings);
  setOverrides(settings.shortcuts);
  const appliedVars = useRef<string[]>([]);
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

  /** Tabs for the core's sessions this window doesn't show: those it had before it closed, an agent's. */
  const restoreSessions = useCallback(async () => {
    const list = await api.sessionsList().catch(() => []);
    const hosts = wsRef.current?.hosts ?? [];
    setSessions((tabs) => {
      const shown = (id: string) => tabs.some((t) => t.attach === id || (t.layout && panesOf(t.layout).some((p) => p.attach === id)));
      const fresh = list.filter((c) => !c.transient && !shownSessions.has(c.id) && !shown(c.id) && hosts.some((h) => h.id === c.hostId && h.vaultId === c.vaultId));
      if (!fresh.length) return tabs;
      return [
        ...tabs,
        ...fresh.map((c) => {
          const host = hosts.find((h) => h.id === c.hostId)!;
          const agent = c.owner.type === "agent" ? c.owner.agent : undefined;
          return { id: uid("s"), hostId: c.hostId, kind: "terminal" as const, title: host.label, status: "connected" as const, attach: c.id, agent, ...onePane(c.hostId, { attach: c.id }) };
        }),
      ];
    });
  }, []);

  useEffect(() => {
    api.coreInfo().then(setCoreInfo).catch(() => setCoreInfo(null));
    api
      .vaultStatus()
      .then(async (st) => {
        if (!st.initialized) return setPhase("setup");
        // A core another program started doesn't know this window: the master password vouches for it.
        if (!st.unlocked || st.trusted === false) return setPhase("locked");
        const fresh = await loadWorkspace(null);
        wsRef.current = fresh;
        setWs(fresh);
        setPhase("ready");
        void restoreSessions();
        void api.agentsPending().then(setAgentAsks).catch(() => {});
      })
      .catch((e) => {
        if (isCoreError(e) && e.kind === "notTrusted") return setPhase("locked");
        // Anything else shows on the crash screen, not as an empty window.
        setStartError(e);
      });
  }, [restoreSessions]);

  const unlocked = async (view: VaultView | null) => {
    lastSaved.current.clear();
    const fresh = await loadWorkspace(view);
    // Keep tunnel switches and other in-memory state of a workspace that was only locked.
    setWs((prev) => (prev && api.real ? { ...fresh, tunnels: fresh.tunnels.map((t) => prev.tunnels.find((p) => p.id === t.id) ?? t) } : prev ?? fresh));
    wsRef.current = fresh;
    setPhase("ready");
    void restoreSessions();
    void api.agentsPending().then(setAgentAsks).catch(() => {});
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
    const shown = JSON.stringify(dataOf(base, view.id));
    const pending = saved !== undefined && shown !== saved && shown !== received.current.get(view.id);
    const next = applyTeamView(base, view, !pending);
    if (!pending) received.current.set(view.id, JSON.stringify(dataOf(next, view.id)));
    wsRef.current = next;
    // Changed in the meantime: take the data only if this team's part is still the same,
    // else keep those edits and take only members, conflicts and activity.
    setWs((cur) => (cur === base ? next : cur && applyTeamView(cur, view, !pending && JSON.stringify(dataOf(cur, view.id)) === shown)));
    // An edit of a Git team is pushed in the background; look again once it had time to go out.
    if (view.remote?.pending && !view.remote.error) {
      clearTimeout(recheck.current.get(view.id));
      recheck.current.set(view.id, setTimeout(() => api.teamSync(view.id).then(applyViewRef.current).catch(() => {}), 1500));
    }
  }, []);
  applyViewRef.current = applyView;

  /** Takes the personal vault from the core again, unless local edits are still on their way. */
  const refreshPersonal = useCallback(async () => {
    const [view, groupPrefs] = await Promise.all([api.vaultGet(), api.groupPrefs().catch(() => null)]);
    const base = wsRef.current;
    if (!base) return;
    const saved = lastSaved.current.get("personal");
    const shown = JSON.stringify(dataOf(base, "personal"));
    if (saved !== undefined && shown !== saved && shown !== received.current.get("personal")) return;
    const data = fromView(view);
    const others = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId !== "personal");
    const take = (w: Workspace): Workspace => {
      // A tunnel's switch and connections belong to this session, not to the vault.
      const running = new Map(w.tunnels.map((t) => [t.id, t]));
      return {
        ...w,
        hosts: [...data.hosts, ...others(w.hosts)],
        keys: [...data.keys, ...others(w.keys)],
        snippets: [...data.snippets, ...others(w.snippets)],
        tunnels: [...data.tunnels.map((t) => ({ ...t, state: running.get(t.id)?.state ?? t.state, connections: running.get(t.id)?.connections, error: running.get(t.id)?.error })), ...others(w.tunnels)],
        proxies: [...data.proxies, ...others(w.proxies)],
        groups: [...data.groups, ...others(w.groups)],
        groupPrefs: groupPrefs ?? w.groupPrefs,
      };
    };
    const next = take(base);
    received.current.set("personal", JSON.stringify(dataOf(next, "personal")));
    wsRef.current = next;
    // Changed in the meantime: take the vault only if its part is still the same.
    setWs((cur) => (cur === base ? next : cur && JSON.stringify(dataOf(cur, "personal")) === shown ? take(cur) : cur));
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

  // The personal vault's sync: its state now; the core syncs every 20 seconds and says so.
  useEffect(() => {
    if (phase === "ready") api.syncStatus().then(setSync).catch(() => {});
  }, [phase]);
  const syncOn = sync?.enabled ?? false;
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
      // The core's own state, now on screen: nothing to save.
      const sent = received.current.get(v.id);
      received.current.delete(v.id);
      if (json === sent) continue;
      const fail = (e: unknown) => toaster.push({ text: tr().common.saveFailed(errorText(e)), tone: "error" });
      const base = JSON.parse(prev) as typeof data;
      if (v.kind === "personal") {
        api
          .vaultPut(data, base)
          .then(() => {
            if (syncOn) afterPersonalEdit();
            // A typed password is now in the vault: drop it from UI memory without another save.
            const base = wsRef.current;
            if (!base || !(data.hosts.some((h) => h.newPassword || h.newProxyPassword) || data.proxies.some((p) => p.newSecrets))) return;
            const next = {
              ...base,
              hosts: base.hosts.map(({ newPassword: _p, newProxyPassword: _q, ...h }) => h),
              proxies: base.proxies.map(({ newSecrets, ...p }) => (newSecrets ? { ...p, saved: savedAfter(p.saved, newSecrets) } : p)),
            };
            received.current.set(v.id, JSON.stringify(dataOf(next, v.id)));
            wsRef.current = next;
            setWs((cur) => (cur === base ? next : cur));
          })
          .catch(fail);
      } else {
        api.teamPut(v.id, data, base).then(applyView).catch(fail);
      }
    }
  }, [ws, phase, toaster.push, applyView, syncOn, afterPersonalEdit]);

  // Tunnels run in the core: keep switches, connection counts and failures in step with it.
  const applyTunnels = useCallback((list: Awaited<ReturnType<typeof api.tunnelsStatus>>) => {
    const byKey = new Map(list.map((s) => [s.key, s]));
    setWs(
      (w) =>
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
  }, []);
  useEffect(() => {
    if (api.real && phase === "ready") api.tunnelsStatus().then(applyTunnels).catch(() => {});
  }, [phase, applyTunnels]);

  // What the core does without the window (sync, teammates' changes, tunnels, the CLI
  // or an agent at work, locking) reaches it as events.
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  useEffect(
    () =>
      api.onCoreEvent((e) => {
        const ready = phaseRef.current === "ready";
        switch (e.name) {
          case "data.changed":
            if (ready) {
              void refreshPersonal();
              void refreshTeams();
            }
            break;
          case "sync.status":
            if (e.data) setSync(e.data as SyncInfo);
            break;
          case "team.synced":
            if (ready) applyView(e.data as TeamView);
            break;
          case "tunnels.status":
            if (ready) applyTunnels(e.data as Awaited<ReturnType<typeof api.tunnelsStatus>>);
            break;
          case "sessions.changed":
            if (ready) void restoreSessions();
            break;
          case "locked":
            if (ready) setPhase("locked");
            break;
          case "unlocked":
            if (phaseRef.current === "locked") void unlockedRef.current(null);
            break;
          case "trust.requested":
            void api.windowShow();
            setTrustAsk(e.data as { id: number; terminal: { name: string; pid: number; host: string | null } | null });
            break;
          case "agents.requested": {
            const req = e.data as AgentRequest;
            // The answer is given in the window: out of the tray with it.
            void api.windowShow();
            setAgentAsks((list) => (list.some((r) => r.id === req.id) ? list : [...list, req]));
            break;
          }
          case "agents.answered":
            setAgentAsks((list) => list.filter((r) => r.id !== (e.data as { id: string }).id));
            break;
          case "trust.answered":
            setTrustAsk((a) => (a && a.id === (e.data as { id: number }).id ? null : a));
            break;
          case "disconnected":
            // The core went away; the next call starts a new one, locked.
            void api.vaultStatus().then((st) => st.initialized && (!st.unlocked || st.trusted === false) && setPhase("locked")).catch(() => {});
            break;
        }
      }),
    [refreshPersonal, refreshTeams, applyView, applyTunnels, restoreSessions],
  );
  const unlockedRef = useRef(unlocked);
  unlockedRef.current = unlocked;

  // Auto-lock runs in the core, so it locks without a window too: it needs the setting
  // and to know the window is in use.
  useEffect(() => {
    if (phase === "ready") void api.coreSettingsSet(settings.autoLockMinutes).catch(() => {});
  }, [phase, settings.autoLockMinutes]);
  useEffect(() => {
    if (!api.real || phase !== "ready") return;
    let last = 0;
    const touch = () => {
      if (Date.now() - last < 30_000) return;
      last = Date.now();
      void api.activity().catch(() => {});
    };
    const events = ["keydown", "mousedown", "wheel"] as const;
    events.forEach((e) => window.addEventListener(e, touch, true));
    return () => events.forEach((e) => window.removeEventListener(e, touch, true));
  }, [phase]);

  // Text follows the language right away: set it before anything below renders.
  const lang = resolveLang(settings.language);
  if (currentLang() !== lang) setLang(lang);

  // The tray says what the core holds; while it holds anything, closing the window hides it there.
  const liveSessions = sessions.filter((s) => s.kind === "terminal" && s.status === "connected").length;
  const liveTunnels = ws?.tunnels.filter((t) => t.state === "on").length ?? 0;
  useEffect(() => {
    if (phase === "loading") return;
    const t = tr().app.tray;
    const unlocked = phase === "ready";
    const status = [unlocked ? t.unlocked : t.locked, liveSessions ? t.sessions(liveSessions) : "", liveTunnels ? t.tunnels(liveTunnels) : ""].filter(Boolean).join(" · ");
    void api.traySet({ tooltip: `Detour: ${status}`, status, open: t.open, lock: unlocked ? t.lock : null, quit: t.quit, keep: unlocked || liveSessions > 0 || liveTunnels > 0 }).catch(() => {});
  }, [phase, liveSessions, liveTunnels, lang]);
  useEffect(() => {
    setWs((cur) => cur && { ...cur, vaults: cur.vaults.map((v) => (v.kind === "personal" ? { ...v, name: tr().app.personal } : v)) });
  }, [lang]);

  // The themes in use: "system" follows the system's dark or light, terminals follow the app unless told otherwise.
  const [systemDark, setSystemDark] = useState(() => matchMedia("(prefers-color-scheme: dark)").matches);
  useEffect(() => {
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const on = () => setSystemDark(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  const themes = useMemo(() => {
    const fallback = builtinThemes[0]!;
    const appId = settings.theme === SYSTEM ? (systemDark ? "detour-dark" : "detour-light") : settings.theme;
    const app: Theme = findTheme(appId, settings.customThemes) ?? fallback;
    const terminal: Theme = settings.terminalTheme === SAME_AS_APP ? app : findTheme(settings.terminalTheme, settings.customThemes) ?? app;
    return { app, terminal };
  }, [settings.theme, settings.terminalTheme, settings.customThemes, systemDark]);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = isLight(themes.app) ? "light" : "dark";
    const vars = appColors(themes.app);
    for (const k of appliedVars.current) if (!vars || !(k in vars)) root.style.removeProperty(k);
    appliedVars.current = vars ? Object.keys(vars) : [];
    if (vars) for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v);
    root.style.colorScheme = isLight(themes.app) ? "light" : "dark";
    // Around the text of every terminal: the terminal theme's background.
    root.style.setProperty("--term", terminalColors(themes.terminal).background);
  }, [themes]);

  // The webview's own menu (Back, Reload, Print) means nothing here; text fields keep theirs.
  useEffect(() => {
    const block = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest?.("input, textarea, [contenteditable=\"true\"]")) e.preventDefault();
    };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);

  // Shortcuts that work anywhere: what each does is read when it fires.
  const globalKey = useRef<(e: KeyboardEvent) => void>(() => {});
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => globalKey.current(e);
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
        return [...list, { id, hostId, kind, title: same ? `${host.label} (${same + 1})` : host.label, status: "connecting", command, ...(kind === "terminal" ? onePane(hostId, { command }) : {}) }];
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

  /** Closes the tabs `pick` chooses by their place; `keep` comes forward if the open one went. */
  const closeSessions = (pick: (i: number) => boolean, keep?: string) => {
    const next = sessions.filter((_, i) => !pick(i));
    setSessions(next);
    if (!next.some((s) => s.id === activeId) && activeId !== "home") setActiveId(keep && next.some((s) => s.id === keep) ? keep : next.at(-1)?.id ?? "home");
  };
  // The tabs as they are now, for changes with side effects (they stay out of state updaters).
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const patchSession = (id: string, patch: (s: Session) => Partial<Session>) => setSessions((list) => list.map((s) => (s.id === id ? { ...s, ...patch(s) } : s)));

  /** Where a split waits for the host to open in it. */
  const [splitPick, setSplitPick] = useState<{ tab: string; pane: string; dir: SplitDir } | null>(null);
  const splitPane = (tab: string, pane: string, dir: SplitDir, hostId?: string) => {
    const s = sessionsRef.current.find((x) => x.id === tab);
    if (!s?.layout || panesOf(s.layout).length >= MAX_PANES) return;
    const from = panesOf(s.layout).find((p) => p.id === pane);
    if (!from) return;
    const add = { id: uid("p"), hostId: hostId ?? from.hostId };
    patchSession(tab, (x) => ({ layout: insert(x.layout!, pane, dir === "row" ? "right" : "bottom", { pane: add }), focused: add.id }));
  };
  const closePane = (tab: string, pane: string) => {
    const s = sessionsRef.current.find((x) => x.id === tab);
    if (!s?.layout) return;
    const next = removePane(s.layout, pane);
    if (!next) return closeSession(tab);
    const focus = s.focused === pane ? (neighbour(s.layout, pane, "left") ?? neighbour(s.layout, pane, "top") ?? panesOf(next)[0]!.id) : s.focused;
    patchSession(tab, () => ({ layout: next, focused: focus }));
  };
  const moveFocus = (tab: string, pane: string, side: Side) => {
    const s = sessionsRef.current.find((x) => x.id === tab);
    const to = s?.layout && neighbour(s.layout, pane, side);
    if (to) patchSession(tab, () => ({ focused: to }));
  };
  /** A pane out of its tab into a tab of its own, its session as it is. */
  const detachPane = (tab: string, pane: string) => {
    const list = sessionsRef.current;
    const s = list.find((x) => x.id === tab);
    const p = s?.layout && panesOf(s.layout).find((x) => x.id === pane);
    if (!s?.layout || !p) return;
    const rest = removePane(s.layout, pane);
    if (!rest) return;
    const host = ws?.hosts.find((h) => h.id === p.hostId);
    const moved = carried({ pane: p });
    const id = uid("s");
    const at = list.indexOf(s);
    const focus = s.focused === pane ? panesOf(rest)[0]!.id : s.focused;
    setSessions([...list.slice(0, at), { ...s, layout: rest, focused: focus }, { id, hostId: p.hostId, kind: "terminal", title: host?.label ?? s.title, status: "connected", layout: moved, focused: pane }, ...list.slice(at + 1)]);
    setActiveId(id);
  };
  /** The terminal tab `from` put into the tab `to`, on `side` of its pane `pane`. */
  const mergeTab = (from: string, to: string, pane: string, side: Side) => {
    const list = sessionsRef.current;
    const src = list.find((x) => x.id === from);
    const dst = list.find((x) => x.id === to);
    if (!src?.layout || !dst?.layout || from === to || panesOf(src.layout).length + panesOf(dst.layout).length > MAX_PANES) return;
    const moved = carried(src.layout);
    setSessions(list.filter((x) => x.id !== from).map((x) => (x.id === to ? { ...x, layout: insert(x.layout!, pane, side, moved), focused: src.focused } : x)));
    setActiveId(to);
  };
  /** Moves a tab to the place of `before` (null: the end). */
  const moveTab = (id: string, before: string | null) =>
    setSessions((list) => {
      const s = list.find((x) => x.id === id);
      if (!s || before === id) return list;
      const rest = list.filter((x) => x.id !== id);
      const at = before === null ? rest.length : rest.findIndex((x) => x.id === before);
      return [...rest.slice(0, at), s, ...rest.slice(at)];
    });

  /** A tab being dragged: to a new place in the row, or (a terminal) into a pane of the tab in front. */
  const [tabDrag, setTabDrag] = useState<{ id: string; drop: { pane: string; side: Side; rect: { left: number; top: number; width: number; height: number } } | null } | null>(null);
  const draggedTab = useRef(false);
  const startTabDrag = (id: string, e: React.MouseEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest(".tab-close")) return;
    const [x0, y0] = [e.clientX, e.clientY];
    let on = false;
    let drop: NonNullable<typeof tabDrag>["drop"] = null;
    const move = (ev: MouseEvent) => {
      if (!on && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      on = true;
      draggedTab.current = true;
      document.body.classList.add("dragging-tab");
      const el = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null;
      const over = el?.closest<HTMLElement>("[data-tab-id]");
      drop = null;
      if (over && over.dataset.tabId !== id) {
        const r = over.getBoundingClientRect();
        const list = sessionsRef.current;
        const i = list.findIndex((x) => x.id === over.dataset.tabId);
        moveTab(id, ev.clientX < r.left + r.width / 2 ? over.dataset.tabId! : list[i + 1]?.id ?? null);
      }
      const paneEl = el?.closest<HTMLElement>(".view:not(.hidden) [data-pane]");
      const src = sessionsRef.current.find((x) => x.id === id);
      if (paneEl && src?.layout && !paneEl.closest(`[data-view="${id}"]`)) {
        const r = paneEl.getBoundingClientRect();
        const [px, py] = [(ev.clientX - r.left) / r.width, (ev.clientY - r.top) / r.height];
        const side = ([["left", px], ["right", 1 - px], ["top", py], ["bottom", 1 - py]] as const).reduce((a, b) => (b[1] < a[1] ? b : a))[0];
        const half = side === "left" || side === "right" ? { width: r.width / 2, height: r.height, top: r.top, left: side === "left" ? r.left : r.left + r.width / 2 } : { width: r.width, height: r.height / 2, left: r.left, top: side === "top" ? r.top : r.top + r.height / 2 };
        drop = { pane: paneEl.dataset.pane!, side, rect: half };
      }
      setTabDrag({ id, drop });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.classList.remove("dragging-tab");
      setTabDrag(null);
      if (on && drop) {
        const target = sessionsRef.current.find((x) => x.id === activeIdRef.current);
        if (target) mergeTab(id, target.id, drop.pane, drop.side);
      }
      // The click that ends a drag doesn't open the tab.
      setTimeout(() => (draggedTab.current = false));
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;

  globalKey.current = (e) => {
    if (recording.on || phase !== "ready") return;
    const take = (run: () => void) => {
      e.preventDefault();
      e.stopPropagation();
      run();
    };
    if (hit(e, "palette")) return take(() => setPalette((p) => !p));
    if (palette || modalOpen()) return;
    if (hit(e, "settings")) return take(() => (setSection("settings"), setActiveId("home")));
    if (hit(e, "lock") && api.real) return take(() => void lock());
    const order = ["home", ...sessions.map((s) => s.id)];
    const at = order.indexOf(activeId);
    if (hit(e, "nextTab")) return take(() => setActiveId(order[(at + 1) % order.length]!));
    if (hit(e, "prevTab")) return take(() => setActiveId(order[(at - 1 + order.length) % order.length]!));
    const cur = sessions.find((s) => s.id === activeId);
    if (!cur) return;
    if (hit(e, "closeTab")) return take(() => (cur.layout && !isPane(cur.layout) && cur.focused ? closePane(cur.id, cur.focused) : closeSession(cur.id)));
    const i = sessions.indexOf(cur);
    if (hit(e, "moveTabLeft")) return take(() => i > 0 && moveTab(cur.id, sessions[i - 1]!.id));
    if (hit(e, "moveTabRight")) return take(() => i < sessions.length - 1 && moveTab(cur.id, sessions[i + 2]?.id ?? null));
  };

  const [tabMenu, setTabMenu] = useState<{ x: number; y: number; id: string } | null>(null);
  const tabMenuItems = (id: string): MenuItem[] => {
    const at = sessions.findIndex((s) => s.id === id);
    const t = tr().app.tabs;
    const s = sessions[at]!;
    const pane = s.layout && s.focused;
    const split = s.layout && pane && panesOf(s.layout).length < MAX_PANES
      ? [
          { label: tr().terminal.menu.splitRight, icon: Columns2, hint: keysOf("splitRight"), onSelect: () => splitPane(id, pane, "row") },
          { label: tr().terminal.menu.splitDown, icon: Rows2, hint: keysOf("splitDown"), onSelect: () => splitPane(id, pane, "col") },
          { label: tr().terminal.menu.splitRightWith, icon: Columns2, onSelect: () => setSplitPick({ tab: id, pane, dir: "row" }) },
          { label: tr().terminal.menu.splitDownWith, icon: Rows2, onSelect: () => setSplitPick({ tab: id, pane, dir: "col" }) },
        ]
      : [];
    return [
      ...split,
      { label: t.close, hint: activeId === id ? keysOf("closeTab") : undefined, sepBefore: split.length > 0, onSelect: () => closeSession(id) },
      ...(sessions.length > 1 ? [{ label: t.closeOthers, onSelect: () => closeSessions((i) => i !== at, id) }] : []),
      ...(at > 0 ? [{ label: t.closeLeft, onSelect: () => closeSessions((i) => i < at, id) }] : []),
      ...(at < sessions.length - 1 ? [{ label: t.closeRight, onSelect: () => closeSessions((i) => i > at, id) }] : []),
      ...(sessions.length > 1 ? [{ label: t.closeAll, danger: true, onSelect: () => closeSessions(() => true) }] : []),
    ];
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
      themes,
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
  }, [ws, vaultId, section, settings, themes, coreInfo, connect, openFiles, toaster.push, lock, applyView, sync, refreshPersonal, updateState, checkUpdate, installUpdate]);

  if (startError) throw startError instanceof Error ? startError : new Error(errorText(startError));
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
        <TipLayer />
        <div className={`app ${settings.sidebarCollapsed ? "sidebar-mini" : ""}`} inert={phase !== "ready"}>
          <Sidebar homeActive={activeId === "home"} />

          <main className="main">
            <div
              className="tabbar"
              onWheel={(e) => {
                // A plain wheel scrolls the tabs sideways, as Shift with it does.
                if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) e.currentTarget.scrollLeft += e.deltaY;
              }}
            >
                <HomeTab section={section} personal={store.vault.kind === "personal"} active={activeId === "home"} onClick={() => setActiveId("home")} />
                {sessions.map((s) => (
                  <div
                    key={s.id}
                    role="tab"
                    tabIndex={0}
                    data-tab-id={s.id}
                    aria-selected={activeId === s.id}
                    className={`tab ${activeId === s.id ? "active" : ""} ${tabDrag?.id === s.id ? "dragging" : ""}`}
                    title={s.agent && tr().agents.tab(s.agent)}
                    onMouseDown={(e) => startTabDrag(s.id, e)}
                    onClick={() => !draggedTab.current && setActiveId(s.id)}
                    onAuxClick={(e) => e.button === 1 && closeSession(s.id)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setTabMenu({ x: e.clientX, y: e.clientY, id: s.id });
                    }}
                  >
                    <span className={`session-dot ${s.status}`} />
                    {s.kind === "files" && <FolderOpen size={13} className="tab-kind" />}
                    {s.agent && <Bot size={13} className="tab-kind" />}
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
                {section === "proxies" && <ProxiesView />}
                {section === "team" && <TeamsView />}
                {section === "settings" && <SettingsView />}
              </div>
              {sessions.map((s) => {
                const host = ws!.hosts.find((h) => h.id === s.hostId) ?? null;
                if (s.kind === "terminal" && s.layout && s.focused) {
                  return (
                    <div key={s.id} data-view={s.id} className={`view ${activeId === s.id ? "" : "hidden"}`}>
                      <TerminalTab
                        layout={s.layout}
                        focused={s.focused}
                        active={activeId === s.id}
                        agent={s.agent}
                        agentSession={s.attach}
                        onLayout={(fn) => patchSession(s.id, (x) => ({ layout: fn(x.layout!) }))}
                        onFocus={(pane) => patchSession(s.id, () => ({ focused: pane }))}
                        onStatus={(st) => setStatus(s.id, st)}
                        onSplit={(pane, dir, other) => (other ? setSplitPick({ tab: s.id, pane, dir }) : splitPane(s.id, pane, dir))}
                        onClosePane={(pane) => closePane(s.id, pane)}
                        onMoveFocus={(pane, side) => moveFocus(s.id, pane, side)}
                        onDetach={(pane) => detachPane(s.id, pane)}
                      />
                    </div>
                  );
                }
                if (!host && !(s.kind === "files" && s.hostId === null)) return null;
                return (
                  <div key={s.id} data-view={s.id} className={`view ${activeId === s.id ? "" : "hidden"}`}>
                    <FilesView host={host} active={activeId === s.id} onStatus={(st) => setStatus(s.id, st)} onClose={() => closeSession(s.id)} onPickHost={(hostId) => pickHost(s.id, hostId)} />
                  </div>
                );
              })}
              {tabDrag?.drop && <div className="pane-drop" style={tabDrag.drop.rect} />}
            </div>
          </main>
        </div>

        {tabMenu && sessions.some((s) => s.id === tabMenu.id) && <ContextMenu at={tabMenu} items={tabMenuItems(tabMenu.id)} onClose={() => setTabMenu(null)} />}
        {palette && <CommandPalette onClose={() => setPalette(false)} />}
        {splitPick && (
          <CommandPalette
            onClose={() => setSplitPick(null)}
            onPickHost={(hostId) => {
              splitPane(splitPick.tab, splitPick.pane, splitPick.dir, hostId);
              setActiveId(splitPick.tab);
            }}
          />
        )}
        {conflictVault && conflictVault.conflicts.length > 0 && <ConflictDialog vault={conflictVault} onClose={() => setConflictVaultId(null)} />}
        {trustAsk && (
          <Modal
            title={tr().app.trustTerminal.title}
            onClose={() => {
              void api.trustAnswer(trustAsk.id, false);
              setTrustAsk(null);
            }}
            onSubmit={() => {
              void api.trustAnswer(trustAsk.id, true);
              setTrustAsk(null);
            }}
            footer={
              <>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => {
                    void api.trustAnswer(trustAsk.id, false);
                    setTrustAsk(null);
                  }}
                >
                  {tr().app.trustTerminal.deny}
                </Button>
                <Button type="submit" variant="primary" autoFocus>
                  {tr().app.trustTerminal.allow}
                </Button>
              </>
            }
          >
            {trustAsk.terminal && (
              <div className="modal-text mono">
                {tr().app.trustTerminal.shell(trustAsk.terminal.name, trustAsk.terminal.host, trustAsk.terminal.pid)}
              </div>
            )}
          </Modal>
        )}
        {!trustAsk && phase === "ready" && agentAsks[0] && (
          <AgentRequestDialog key={agentAsks[0].id} request={agentAsks[0]} onDone={() => setAgentAsks((list) => list.slice(1))} />
        )}
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
