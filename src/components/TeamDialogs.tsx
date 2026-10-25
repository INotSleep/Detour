import { useEffect, useState } from "react";
import { Check, Copy, Download, ExternalLink, FolderOpen, KeyRound, Loader2, ShieldAlert, ShieldCheck, ShieldQuestion } from "lucide-react";
import { api, errorText, isCoreError, type GitLogin, type GithubCode, type TeamView } from "../lib/api";
import { copyText, sleep, uid, useStore } from "../lib/store";
import type { Level, Role, SshKey, Vault } from "../lib/types";
import { Button, Field, IconButton, Modal, PasswordInput, Segmented } from "./ui";
import { tr, useT } from "../i18n";

/* ---------- demo stand-ins (plain browser) ---------- */

const demoChars = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const demoGroup = () => Array.from({ length: 4 }, () => demoChars[Math.floor(Math.random() * demoChars.length)]).join("");
const demoRecoveryKey = () => `DETOUR-RECOVERY-${Array.from({ length: 28 }, demoGroup).join("-")}`;
const DEMO_ID = { detourId: "detour1q8m3x5r7k2w9t4v6n1p0s8d3f7g2h5j9k4l6z1c8b3n7m2q5w0e4r9t6y1u3i8o5p2a7s4d6f9g1h3j8k5l0z2x7c4v9b6", fingerprint: "K7QM-2XPA-9DTE-WN4F" };

export function useMyId() {
  const [id, setId] = useState<{ detourId: string; fingerprint: string } | null>(api.real ? null : DEMO_ID);
  useEffect(() => {
    if (api.real) api.myId().then(setId).catch(() => setId(null));
  }, []);
  return id;
}

export function FolderField({ value, onChange, label, placeholder = "D:\\Sync\\team-vault" }: { value: string; onChange: (v: string) => void; label?: string; placeholder?: string }) {
  const t = useT();
  return (
    <Field label={label ?? t.teams.storage.folder}>
      <div className="input-group">
        <input className="input mono" value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} spellCheck={false} />
        {api.real && (
          <Button type="button" icon={FolderOpen} onClick={async () => { const p = await api.pickFolder(); if (p) onChange(p); }}>
            {t.teams.storage.choose}
          </Button>
        )}
      </div>
    </Field>
  );
}

/* ---------- where a team vault lives: a folder or a Git repository ---------- */

export interface Storage {
  kind: "folder" | "git";
  path: string;
  url: string;
  /** Personal key id, or "agent". */
  login: string;
  /** HTTPS access token. */
  token: string;
  /** A github.com repository: a token instead of the GitHub sign-in. */
  useToken: boolean;
  /** The GitHub sign-in reaches the repository. */
  githubReady: boolean;
}

export function useStorage(): [Storage, (patch: Partial<Storage>) => void] {
  const { ws } = useStore();
  const firstKey = ws.keys.find((k) => k.vaultId === "personal")?.id ?? "agent";
  const [s, setS] = useState<Storage>({ kind: "folder", path: "", url: "", login: firstKey, token: "", useToken: false, githubReady: false });
  return [s, (patch) => setS((cur) => ({ ...cur, ...patch }))];
}

const isHttps = (url: string) => /^https?:\/\//i.test(url.trim());
const viaGithub = (s: Storage) => !!githubRepo(s.url) && !s.useToken;
export const gitLogin = (s: Storage): GitLogin =>
  viaGithub(s)
    ? { url: s.url.trim(), auth: { type: "password" }, github: true }
    : isHttps(s.url)
      ? { url: s.url.trim(), auth: { type: "password" }, token: s.token.trim() }
      : { url: s.url.trim(), auth: s.login === "agent" ? { type: "agent" } : { type: "key", keyId: s.login } };
export const storageReady = (s: Storage) =>
  s.kind === "folder" ? !!s.path.trim() : !!s.url.trim() && (viaGithub(s) ? s.githubReady : !isHttps(s.url) || !!s.token.trim());

export function StorageFields({ value, set, folderLabel, folderPlaceholder }: { value: Storage; set: (p: Partial<Storage>) => void; folderLabel: string; folderPlaceholder?: string }) {
  const t = useT();
  const { ws, toast, update } = useStore();
  const keys = ws.keys.filter((k) => k.vaultId === "personal");
  const key = keys.find((k) => k.id === value.login);
  const [creating, setCreating] = useState(false);
  // A key made here still has to be added to the repository: show it until then.
  const [created, setCreated] = useState<string | null>(null);

  const createKey = async () => {
    setCreating(true);
    try {
      const name = repoName(value.url) ?? "git";
      const k = await api.keyGenerate("personal", name, "ed25519", null, true);
      update((w) => ({ ...w, keys: [...w.keys, k] }));
      setCreated(k.id);
      set({ login: k.id });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    } finally {
      setCreating(false);
    }
  };
  const copyPublic = async (k: SshKey) =>
    toast((await copyText(k.publicKey)) ? { text: t.teams.storage.publicKeyCopied(k.name), tone: "success" } : { text: t.common.copyFailed, tone: "error" });
  return (
    <>
      <Field label={t.teams.storage.title}>
        <Segmented value={value.kind} onChange={(kind) => set({ kind })} options={[{ value: "folder", label: t.teams.storage.folder }, { value: "git", label: t.teams.storage.git }]} />
      </Field>
      {value.kind === "folder" ? (
        <FolderField value={value.path} onChange={(path) => set({ path })} label={folderLabel} placeholder={folderPlaceholder} />
      ) : (
        <>
          <Field label={t.teams.storage.url}>
            <input className="input mono" value={value.url} onChange={(e) => set({ url: e.target.value })} placeholder="git@github.com:team/vault.git" spellCheck={false} autoComplete="off" />
          </Field>
          {viaGithub(value) ? (
            <>
              <GithubAccess url={value.url} onReady={(githubReady) => set({ githubReady })} />
              <Button type="button" variant="ghost" size="sm" className="self-start" onClick={() => set({ useToken: true })}>{t.teams.storage.useToken}</Button>
            </>
          ) : isHttps(value.url) ? (
            <>
              <TokenField label={t.teams.storage.token} repoUrl={value.url} value={value.token} onChange={(token) => set({ token })} placeholder="github_pat_…" />
              {githubRepo(value.url) && (
                <Button type="button" variant="ghost" size="sm" className="self-start" onClick={() => set({ useToken: false })}>{t.teams.storage.useGithub}</Button>
              )}
            </>
          ) : (
          <>
          <Field label={t.teams.storage.login}>
            <div className="input-group">
              <select className="input" value={creating ? "new" : value.login} disabled={creating} onChange={(e) => (e.target.value === "new" ? void createKey() : set({ login: e.target.value }))}>
                {keys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
                <option value="new">{creating ? t.teams.storage.creatingKey : t.teams.storage.newKey}</option>
                <option value="agent">ssh-agent</option>
              </select>
              {key && key.id !== created && (
                <Button type="button" icon={Copy} onClick={() => void copyPublic(key)}>
                  {t.teams.storage.publicKey}
                </Button>
              )}
            </div>
          </Field>
          {key && key.id === created && (
            <div className="deploy-key">
              <div className="input-group">
                <div className="id-box mono grow">{key.publicKey}</div>
                <IconButton type="button" icon={Copy} label={t.common.copy} onClick={() => void copyPublic(key)} />
              </div>
              <span className="muted small">{t.teams.storage.deployKeyHint}</span>
            </div>
          )}
          </>
          )}
        </>
      )}
    </>
  );
}

/** A GitHub repository given by its https URL. */
export const githubRepo = (url: string) => {
  const m = /^https:\/\/github\.com\/([A-Za-z0-9-]+)\/([A-Za-z0-9._-]+?)(?:\.git)?\/?$/i.exec(url.trim());
  return m ? { owner: m[1], repo: m[2] } : null;
};

/** GitHub's fine-grained token form, filled in for one repository: write access to its contents. */
const githubTokenForm = (owner: string, repo: string) => {
  // Token names must be unique on an account, so a renewed token gets today's date.
  const name = `Detour ${repo}`.slice(0, 29) + ` ${new Date().toISOString().slice(0, 10)}`;
  const q = new URLSearchParams({ name, description: `${owner}/${repo}`, target_name: owner, expires_in: "366", contents: "write" });
  return `https://github.com/settings/personal-access-tokens/new?${q}`;
};

/** A Git error about the GitHub sign-in or the Detour Sync app, which signing in or installing fixes. */
export const githubIssue = (e: { kind: string; message?: string } | null | undefined) =>
  e?.kind === "git" && /github sign-in needed|github app has no access/.test(e.message ?? "");

/**
 * A new login for an https repository: the GitHub sign-in for a github.com
 * repository (or a token instead), a token for others. `submit` saves it.
 */
export function RepoLoginFields({ url, token, setToken, error, setError, useToken, setUseToken, onGithubReady }: { url: string; token: string; setToken: (v: string) => void; error: string | null; setError: (e: string | null) => void; useToken: boolean; setUseToken: (v: boolean) => void; onGithubReady: (ready: boolean) => void }) {
  const t = useT();
  if (githubRepo(url) && !useToken) {
    return (
      <>
        <GithubAccess url={url} onReady={onGithubReady} />
        {error && <span className="field-error">{error}</span>}
        <Button type="button" variant="ghost" size="sm" className="self-start" onClick={() => setUseToken(true)}>{t.teams.storage.useToken}</Button>
      </>
    );
  }
  return (
    <>
      <TokenField label={t.teams.token.label} repoUrl={url} value={token} onChange={(v) => { setToken(v); setError(null); }} error={error} autoFocus />
      {githubRepo(url) && <Button type="button" variant="ghost" size="sm" className="self-start" onClick={() => setUseToken(false)}>{t.teams.storage.useGithub}</Button>}
    </>
  );
}

/** An HTTPS token input; for a GitHub repository it also opens GitHub's token form, filled in. */
export function TokenField({ label, repoUrl, value, onChange, error, autoFocus, placeholder }: { label: string; repoUrl: string; value: string; onChange: (v: string) => void; error?: string | null; autoFocus?: boolean; placeholder?: string }) {
  const t = useT();
  const { toast } = useStore();
  const gh = githubRepo(repoUrl);
  const [opened, setOpened] = useState(false);
  const openForm = async () => {
    if (!gh) return;
    try {
      await api.openInBrowser(githubTokenForm(gh.owner, gh.repo));
      setOpened(true);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };
  return (
    <Field label={label} error={error}>
      <div className="input-group">
        <PasswordInput value={value} onChange={onChange} invalid={!!error} autoFocus={autoFocus} placeholder={placeholder} />
        {gh && (
          <Button type="button" icon={ExternalLink} onClick={() => void openForm()}>
            {t.teams.storage.createToken}
          </Button>
        )}
      </div>
      {gh && opened && !value.trim() && <span className="muted small">{t.teams.storage.tokenHint(`${gh.owner}/${gh.repo}`)}</span>}
    </Field>
  );
}

const INSTALL_URL = "https://github.com/apps/detour-sync/installations/new";

/**
 * This device's GitHub sign-in for one github.com repository: signing in with
 * a code entered on GitHub, then whether the Detour Sync app reaches the
 * repository, with a way to install it there. `onReady` follows "it does".
 */
export function GithubAccess({ url, onReady }: { url: string; onReady: (ready: boolean) => void }) {
  const t = useT();
  const g = t.teams.github;
  const { toast } = useStore();
  const repo = githubRepo(url);
  const [user, setUser] = useState<string | null | undefined>(undefined);
  const [code, setCode] = useState<GithubCode | null>(null);
  const [starting, setStarting] = useState(false);
  const [access, setAccess] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.githubUser().then(setUser).catch(() => setUser(null));
  }, []);

  // Waits for the code to be entered on GitHub, as often as GitHub allows.
  useEffect(() => {
    if (!code) return;
    let stop = false;
    let interval = code.interval;
    let timer: ReturnType<typeof setTimeout>;
    const next = () => {
      timer = setTimeout(async () => {
        try {
          const r = await api.githubPoll(code.deviceCode);
          if (stop) return;
          if (r.status === "done") {
            setCode(null);
            setUser(r.user);
            toast({ text: g.signedIn(r.user), tone: "success" });
            return;
          }
          if (r.status === "slowDown") interval = r.interval;
          next();
        } catch (e) {
          if (stop) return;
          setCode(null);
          setError(errorText(e));
        }
      }, interval * 1000);
    };
    next();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [code]);

  // Whether the app reaches the repository; while it doesn't, checks again now and then (after installing it).
  const repoKey = repo ? `${repo.owner}/${repo.repo}` : "";
  useEffect(() => {
    setAccess(null);
    if (!user || !repo) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      try {
        const ok = await api.githubRepoAccess(url);
        if (stop) return;
        setAccess(ok);
        setError(null);
        if (!ok) timer = setTimeout(check, 4000);
      } catch (e) {
        if (stop) return;
        setError(errorText(e));
        if (isCoreError(e) && e.kind === "git" && e.message.includes("sign-in needed")) setUser(null);
      }
    };
    void check();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [user, repoKey]);

  useEffect(() => onReady(!!user && access === true), [user, access]);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      setCode(await api.githubStart());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setStarting(false);
    }
  };
  const copyAndOpen = async (c: GithubCode) => {
    await copyText(c.userCode);
    try {
      await api.openInBrowser(c.verificationUri);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };
  const install = async () => {
    try {
      await api.openInBrowser(INSTALL_URL);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  if (!repo || user === undefined) return null;
  return (
    <div className="field">
      <span className="field-label">GitHub</span>
      {code ? (
        <div className="github-code">
          <div className="input-group">
            <div className="id-box mono grow github-user-code">{code.userCode}</div>
            <Button type="button" variant="primary" icon={ExternalLink} onClick={() => void copyAndOpen(code)}>{g.copyOpen}</Button>
          </div>
          <span className="muted small github-status"><Loader2 size={13} className="spin" />{g.waiting}</span>
        </div>
      ) : !user ? (
        <Button type="button" variant="primary" className="self-start" loading={starting} onClick={() => void start()}>{g.signIn}</Button>
      ) : (
        <div className="github-account">
          <span className="mono small">{g.account(user)}</span>
          {access === null && !error && <span className="muted small github-status"><Loader2 size={13} className="spin" />{g.checking}</span>}
          {access === true && <span className="small github-status ok"><Check size={13} />{g.hasAccess(repoKey)}</span>}
          {access === false && (
            <div className="github-install">
              <span className="small github-status warn"><ShieldAlert size={13} />{g.noAccess(repoKey)}</span>
              <Button type="button" size="sm" icon={ExternalLink} onClick={() => void install()}>{g.install}</Button>
            </div>
          )}
        </div>
      )}
      {error && <span className="field-error">{error}</span>}
    </div>
  );
}

/** The repository name in a Git URL, to name a key made for it. */
const repoName = (url: string) => url.trim().replace(/\.git$/, "").split(/[/:]/).pop()?.replace(/[^A-Za-z0-9._-]/g, "") || null;

/** The server key of a repository seen for the first time. */
export function ServerKeyCard({ url, fingerprint }: { url: string; fingerprint: string }) {
  const t = useT();
  const host = url.replace(/^ssh:\/\//, "").replace(/^[^@]*@/, "").split(/[:/]/)[0];
  return (
    <div className="trust-card">
      <ShieldQuestion size={18} />
      <div>
        <div className="strong">{t.teams.storage.newServer(host)}</div>
        <div className="fp mono">{fingerprint}</div>
        <div className="muted small">{t.teams.storage.checkServer}</div>
      </div>
    </div>
  );
}

/** A new server key from an error, if that is what it is. */
export const unknownServerKey = (e: unknown) => (isCoreError(e) && e.kind === "hostKeyUnknown" ? e.fingerprint : null);

/* ---------- my Detour ID ---------- */

export function MyIdDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { toast } = useStore();
  const me = useMyId();
  return (
    <Modal title={t.teams.myId.title} width={480} onClose={onClose} footer={<Button variant="primary" icon={Copy} onClick={async () => { if (me && (await copyText(me.detourId))) toast({ text: t.teams.page.idCopied, tone: "success" }); }}>{t.common.copy}</Button>}>
      <div className="id-box mono">{me?.detourId ?? "…"}</div>
      <div className="fp-row">
        <span className="muted small">{t.teams.myId.fingerprint}</span>
        <span className="fp mono">{me?.fingerprint ?? "…"}</span>
      </div>
    </Modal>
  );
}

/* ---------- create, with a mandatory recovery key ---------- */

export function CreateTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
  const t = useT();
  const { update, setVault, toast, applyTeamView } = useStore();
  const [step, setStep] = useState<"details" | "recovery">("details");
  const [name, setName] = useState("");
  const [storage, setStorage] = useStorage();
  const [myName, setMyName] = useState("");
  const [recovery, setRecovery] = useState("");
  const [confirm, setConfirm] = useState("");
  const [serverKey, setServerKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const lastFour = recovery.replace(/[^A-Z0-9]/gi, "").slice(-4).toUpperCase();
  const confirmed = confirm.replace(/[^A-Z0-9]/gi, "").toUpperCase() === lastFour;
  const detailsOk = name.trim() && storageReady(storage) && myName.trim();

  const next = async () => {
    if (!detailsOk) return;
    setRecovery(api.real ? await api.recoveryKeyNew() : demoRecoveryKey());
    setStep("recovery");
  };

  const create = async () => {
    if (!confirmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (api.real) {
        const view: TeamView =
          storage.kind === "git"
            ? await api.teamCreateGit(gitLogin(storage), name.trim(), myName.trim(), recovery, serverKey ?? undefined)
            : await api.teamCreate(storage.path.trim(), name.trim(), myName.trim(), recovery);
        applyTeamView(view);
        setVault(view.id);
      } else {
        await sleep(500);
        const id = uid("v");
        const meId = uid("m");
        const sync = storage.kind === "git" ? { type: "git" as const, remote: storage.url.trim() } : { type: "folder" as const, path: storage.path.trim() };
        const vault: Vault = { id, name: name.trim(), kind: "team", sync, syncStatus: "synced", lastSync: Date.now(), conflicts: [], joined: true, myRole: "owner", me: meId, problems: [] };
        update((w) => ({
          ...w,
          vaults: [...w.vaults, vault],
          members: [
            ...w.members,
            { id: meId, vaultId: id, name: myName.trim(), role: "owner", recovery: false, fingerprint: DEMO_ID.fingerprint, isYou: true },
            { id: uid("m"), vaultId: id, name: "Recovery key", role: "owner", recovery: true, fingerprint: demoGroup() + "-" + demoGroup(), isYou: false },
          ],
          activity: [...w.activity, { vaultId: id, authorName: myName.trim(), isYou: true, at: Date.now(), kind: "created" }],
        }));
        setVault(id);
      }
      onDone?.();
      toast({ text: t.teams.create.created(name.trim()), tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      const fp = unknownServerKey(e);
      if (fp) setServerKey(fp);
      else setError(isCoreError(e) && e.kind === "folderNotEmpty" ? (storage.kind === "git" ? t.teams.create.repoNotEmpty : t.common.errors.folderNotEmpty) : errorText(e));
    }
  };

  if (step === "details") {
    return (
      <Modal
        title={t.teams.create.title}
        width={520}
        onClose={onClose}
        onSubmit={next}
        footer={
          <>
            <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
            <Button type="submit" variant="primary" disabled={!detailsOk}>{t.common.next}</Button>
          </>
        }
      >
        <Field label={t.teams.create.name}>
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Infra" autoFocus />
        </Field>
        <StorageFields value={storage} set={(p) => { setStorage(p); setServerKey(null); }} folderLabel={t.teams.create.emptyFolder} />
        <Field label={t.teams.create.myName}>
          <input className="input" value={myName} onChange={(e) => setMyName(e.target.value)} placeholder={t.teams.create.myNamePlaceholder} />
        </Field>
      </Modal>
    );
  }

  return (
    <Modal
      title={t.teams.create.recoveryTitle}
      width={540}
      onClose={onClose}
      onSubmit={create}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => setStep("details")}>{t.common.back}</Button>
          <Button type="submit" variant="primary" disabled={!confirmed} loading={busy}>{serverKey ? t.teams.create.trustAndCreate : t.teams.create.create}</Button>
        </>
      }
    >
      <div className="modal-text">{t.teams.create.recoveryHint}</div>
      <div className="recovery-box mono">{recovery}</div>
      <div className="row-buttons">
        <Button type="button" icon={Copy} onClick={async () => { if (await copyText(recovery)) { toast({ text: t.teams.create.keyCopied, tone: "success" }); } }}>{t.common.copy}</Button>
        <Button
          type="button"
          icon={Download}
          onClick={async () => {
            const where = await api.saveTextFile(`detour-recovery-${name.trim().toLowerCase().replace(/\s+/g, "-") || "team"}.txt`, `${name.trim()}\n${recovery}\n`);
            if (where) toast({ text: t.teams.create.saved(where), tone: "success" });
          }}
        >
          {t.teams.create.saveToFile}
        </Button>
      </div>
      <Field label={t.teams.create.lastFour} error={confirm.length >= 4 && !confirmed ? t.teams.create.mismatch : error}>
        <input className={`input mono confirm-input ${confirm.length >= 4 && !confirmed ? "invalid" : ""}`} value={confirm} onChange={(e) => setConfirm(e.target.value.slice(0, 4))} placeholder="••••" autoComplete="off" spellCheck={false} />
      </Field>
      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}
    </Modal>
  );
}

/* ---------- join (or recover) ---------- */

export function JoinTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
  const t = useT();
  const { setVault, toast, applyTeamView } = useStore();
  const me = useMyId();
  const [mode, setMode] = useState<"join" | "recover">("join");
  const [storage, setStorageRaw] = useStorage();
  const [info, setInfo] = useState<{ root: string; rootFingerprint: string } | null>(null);
  const [clone, setClone] = useState<string | null>(null);
  const [serverKey, setServerKey] = useState<string | null>(null);
  const [recoveryKey, setRecoveryKey] = useState("");
  const [myName, setMyName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const setStorage = (p: Partial<Storage>) => {
    setStorageRaw(p);
    setInfo(null);
    setClone(null);
    setServerKey(null);
    setError(null);
  };

  const finish = (view: TeamView) => {
    applyTeamView(view);
    setVault(view.id);
    onDone?.();
    toast({ text: view.joined ? t.teams.join.joined(view.name) : t.teams.join.waiting, tone: "success" });
    onClose();
  };

  const submit = async () => {
    if (busy || !storageReady(storage)) return;
    if (!api.real) {
      toast({ text: t.teams.join.appOnly, tone: "error" });
      return;
    }
    setBusy(true);
    setError(null);
    const trust = serverKey ?? undefined;
    try {
      if (storage.kind === "git") {
        const login = gitLogin(storage);
        if (mode === "recover") return finish(await api.teamRecoverGit(login, recoveryKey, myName.trim() || "Owner", trust));
        if (!info || !clone) {
          const found = await api.teamInspectGit(login, trust);
          setInfo(found.info);
          setClone(found.clone);
          setServerKey(null);
          setBusy(false);
          return;
        }
        return finish(await api.teamJoinGit(clone, info.root, login));
      }
      const path = storage.path.trim();
      if (mode === "recover") return finish(await api.teamRecover(path, recoveryKey, myName.trim() || "Owner"));
      if (!info) {
        setInfo(await api.teamInspect(path));
        setBusy(false);
        return;
      }
      finish(await api.teamJoin(path, info.root));
    } catch (e) {
      setBusy(false);
      const fp = unknownServerKey(e);
      if (fp) setServerKey(fp);
      else setError(isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? t.teams.remote.unreachable : t.teams.page.folderUnavailable) : errorText(e));
    }
  };

  const title = mode === "recover" ? t.teams.join.recoverTitle : t.teams.join.joinTitle;
  const action = serverKey ? t.teams.join.trustServer : mode === "recover" ? t.teams.join.recover : info ? t.teams.join.trustAndJoin : t.common.next;

  return (
    <Modal
      title={title}
      width={540}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" className="footer-left" onClick={() => { setMode(mode === "join" ? "recover" : "join"); setInfo(null); setClone(null); setError(null); }}>
            {mode === "join" ? t.teams.join.haveRecoveryKey : t.teams.join.normalJoin}
          </Button>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!storageReady(storage) || (mode === "recover" && !recoveryKey.trim())}>{action}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel={t.teams.join.folder} />

      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}

      {mode === "join" && info && (
        <div className="trust-card">
          <ShieldCheck size={18} />
          <div>
            <div className="strong">{t.teams.join.creator}</div>
            <div className="fp mono">{info.rootFingerprint}</div>
            <div className="muted small">{t.teams.join.checkCreator}</div>
          </div>
        </div>
      )}

      {mode === "recover" && (
        <>
          <Field label={t.teams.join.recoveryKey}>
            <textarea className="input mono textarea" rows={3} value={recoveryKey} onChange={(e) => setRecoveryKey(e.target.value)} placeholder="DETOUR-RECOVERY-…" spellCheck={false} />
          </Field>
          <Field label={t.teams.join.myName}>
            <input className="input" value={myName} onChange={(e) => setMyName(e.target.value)} />
          </Field>
        </>
      )}

      {error && <span className="field-error">{error}</span>}
      {mode === "join" && me && (
        <div className="muted small">{t.teams.join.myFingerprint} <span className="mono">{me.fingerprint}</span></div>
      )}
    </Modal>
  );
}

/* ---------- add a member by Detour ID ---------- */

export function AddMemberDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
  const t = useT();
  const { update, toast, applyTeamView } = useStore();
  const [detourId, setDetourId] = useState("");
  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [idError, setIdError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [access, setAccess] = useState<Level>("view");
  const [busy, setBusy] = useState(false);
  const iAmOwner = vault.myRole === "owner";

  useEffect(() => {
    setFingerprint(null);
    setIdError(null);
    const id = detourId.trim();
    if (!id) return;
    const timer = setTimeout(async () => {
      if (!api.real) {
        if (/^detour1[a-z2-7]{100,}$/i.test(id)) setFingerprint(`${demoGroup()}-${demoGroup()}-${demoGroup()}-${demoGroup()}`);
        else setIdError(tr().common.errors.invalidId);
        return;
      }
      try {
        setFingerprint(await api.idInspect(id));
      } catch {
        setIdError(tr().common.errors.invalidId);
      }
    }, 250);
    return () => clearTimeout(timer);
  }, [detourId]);

  const valid = !!fingerprint && name.trim().length > 0;

  const submit = async () => {
    if (!valid || busy) return;
    setBusy(true);
    try {
      if (api.real) {
        applyTeamView(await api.teamMemberAdd(vault.id, detourId.trim(), name.trim(), role, access));
      } else {
        await sleep(400);
        update((w) => ({
          ...w,
          members: [...w.members, { id: uid("m"), vaultId: vault.id, name: name.trim(), role, recovery: false, fingerprint: fingerprint!, isYou: false }],
          activity: [...w.activity, { vaultId: vault.id, authorName: t.common.you, isYou: true, at: Date.now(), kind: "memberAdded", member: name.trim() }],
        }));
      }
      toast({ text: t.teams.activity.memberAdded(name.trim()), tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      if (isCoreError(e) && e.kind === "alreadyMember") setIdError(t.common.errors.alreadyMember);
      else toast({ text: errorText(e), tone: "error" });
    }
  };

  return (
    <Modal
      title={t.teams.add.title(vault.name)}
      width={500}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!valid} loading={busy}>{t.common.add}</Button>
        </>
      }
    >
      <Field label={t.teams.add.detourId} error={idError}>
        <textarea className={`input mono textarea id-input ${idError ? "invalid" : ""}`} rows={3} value={detourId} onChange={(e) => setDetourId(e.target.value)} placeholder="detour1…" spellCheck={false} autoFocus />
      </Field>
      {fingerprint && (
        <div className="fp-row">
          <span className="muted small">{t.teams.add.fingerprint}</span>
          <span className="fp mono">{fingerprint}</span>
        </div>
      )}
      <div className="row">
        <Field label={t.teams.add.name}>
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Anna" />
        </Field>
        <Field label={t.teams.add.role}>
          <select className="input" value={role} onChange={(e) => setRole(e.target.value as Role)}>
            {iAmOwner && <option value="owner">{t.common.role.owner}</option>}
            {iAmOwner && <option value="admin">{t.common.role.admin}</option>}
            <option value="member">{t.common.role.member}</option>
          </select>
        </Field>
      </div>
      <Field label={t.teams.add.access}>
        <Segmented
          value={access}
          onChange={setAccess}
          options={(["none", "view", "secret", "edit"] as const).map((l) => ({ value: l, label: t.teams.level[l] }))}
        />
      </Field>
    </Modal>
  );
}

export function CopyIdButton() {
  const t = useT();
  const { toast } = useStore();
  const me = useMyId();
  return <IconButton icon={KeyRound} label={t.teams.copyMyId} onClick={async () => { if (me && (await copyText(me.detourId))) toast({ text: t.teams.page.idCopied, tone: "success" }); }} />;
}
