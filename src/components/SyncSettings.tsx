import { useEffect, useMemo, useState } from "react";
import { renderSVG } from "uqr";
import { ChevronRight, Cloud, Copy, Download, FolderSync, GitBranch, Laptop, Link2, RotateCw, ShieldAlert } from "lucide-react";
import { api, driveNames, errorText, isCoreError, type CoreError, type DriveFolderRef, type DriveProvider, type Pairing, type SyncInfo, type SyncPlace } from "../lib/api";
import { DriveSignInDialog } from "./DriveFields";
import { locale, tr, useT } from "../i18n";
import { copyText, formatAgo, useStore } from "../lib/store";
import { Button, ConfirmDialog, Field, Modal } from "./ui";
import { RepoLoginFields, ServerKeyCard, StorageFields, githubIssue, githubRepo, gitLogin, storageReady, unknownServerKey, useStorage, type Storage } from "./TeamDialogs";
import { problemText, remoteErrorText } from "./TeamsView";

const placeOf = (s: Storage): SyncPlace => (s.kind === "folder" ? { path: s.path.trim() } : s.kind === "drive" && s.drive ? { drive: s.drive } : gitLogin(s));

/** What went wrong with the last sync, in words. */
export function syncErrorText(s: SyncInfo): string {
  const e = s.error;
  if (!e) return "";
  if ((!s.storage || s.storage === "folder") && !s.git && (e.kind === "unavailable" || e.kind === "io")) return tr().sync.folderUnavailable;
  return remoteErrorText(e);
}

const tokenRejected = (e: CoreError | null) => githubIssue(e) || (e?.kind === "git" && e.message.includes("token rejected"));

/** Settings: syncing the personal vault between this person's devices. */
export function SyncSettings() {
  const { sync, setSync, refreshPersonal, toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [dialog, setDialog] = useState<"enable" | "device" | "recovery" | "disable" | "token" | "serverKey" | "driveSignIn" | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [showProblems, setShowProblems] = useState(false);

  const run = async (go: () => Promise<{ changed: boolean; status: SyncInfo }>) => {
    setSyncing(true);
    try {
      const r = await go();
      setSync(r.status);
      if (r.changed) await refreshPersonal();
      if (r.status.error) toast({ text: syncErrorText(r.status), tone: "error" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    } finally {
      setSyncing(false);
    }
  };

  if (!sync) return null;

  if (!sync.enabled) {
    return (
      <section className="settings-group">
        <h3 className="section-title">{t.title}</h3>
        <div className="setting">
          <span>{t.betweenDevices}</span>
          <div className="row-buttons">
            <Button size="sm" variant="primary" onClick={() => setDialog("enable")}>{t.enable}</Button>
          </div>
        </div>
        {dialog === "enable" && <EnableDialog onClose={() => setDialog(null)} />}
      </section>
    );
  }

  const failed = !!sync.error;
  const keyChanged = sync.error?.kind === "hostKeyChanged";
  const status = failed ? syncErrorText(sync) : syncing ? t.syncing : sync.pending ? t.notSent : sync.syncedAt ? t.syncedAgo(formatAgo(sync.syncedAt)) : t.neverSynced;
  const problems = sync.problems;

  return (
    <section className="settings-group">
      <h3 className="section-title">{t.title}</h3>
      <div className="sync-card">
        <div className="item-icon">{sync.git ? <GitBranch size={15} /> : sync.storage && sync.storage !== "folder" ? <Cloud size={15} /> : <FolderSync size={15} />}</div>
        <div className="sync-main">
          <span className="mono sync-location" title={sync.location ?? ""}>{sync.location}</span>
          <span className={`small ${failed ? "err-text" : sync.pending ? "warn-text" : "muted"}`}>{status}</span>
        </div>
        {keyChanged ? (
          <Button size="sm" onClick={() => setDialog("serverKey")}>{t.replaceServerKey}</Button>
        ) : sync.error?.kind === "driveSignIn" ? (
          <Button size="sm" variant="primary" onClick={() => setDialog("driveSignIn")}>{all.teams.drive.signInFix(driveNames[sync.error.provider as DriveProvider] ?? sync.error.provider)}</Button>
        ) : tokenRejected(sync.error) ? (
          <Button size="sm" onClick={() => setDialog("token")}>{githubIssue(sync.error) ? all.teams.github.fix : t.newToken}</Button>
        ) : (
          <Button size="sm" icon={RotateCw} loading={syncing} onClick={() => void run(() => api.syncNow())}>{t.syncNow}</Button>
        )}
      </div>

      {problems.length > 0 && (
        <div className="banner warn problems">
          <button className="problems-head" onClick={() => setShowProblems((s) => !s)} aria-expanded={showProblems}>
            <ShieldAlert size={16} />
            <span className="grow">{t.storageProblems(all.common.problemCount(problems.length))}</span>
            <ChevronRight size={14} className={`chev ${showProblems ? "open" : ""}`} />
          </button>
          {showProblems && (
            <ul className="problems-list">
              {problems.map((p, i) => <li key={i}>{problemText(p)}</li>)}
            </ul>
          )}
        </div>
      )}

      <div className="setting">
        <span>{t.otherDevice}</span>
        <Button size="sm" icon={Laptop} onClick={() => setDialog("device")}>{all.common.add}</Button>
      </div>
      <div className="setting">
        <span>{t.recoveryKey}</span>
        <Button size="sm" onClick={() => setDialog("recovery")}>{all.common.show}</Button>
      </div>
      <div className="setting">
        <span>{t.onThisDevice}</span>
        <Button size="sm" variant="ghost" className="danger-text" onClick={() => setDialog("disable")}>{t.disable}</Button>
      </div>

      {dialog === "driveSignIn" && sync.error?.kind === "driveSignIn" && (
        <DriveSignInDialog provider={sync.error.provider as DriveProvider} account={sync.error.account} onClose={() => setDialog(null)} onDone={() => { setDialog(null); void run(() => api.syncNow()); }} />
      )}
      {dialog === "device" && <AddDeviceDialog repo={sync.location} drive={sync.drive ?? null} onClose={() => setDialog(null)} />}
      {dialog === "recovery" && <RecoveryDialog onClose={() => setDialog(null)} />}
      {dialog === "token" && <TokenDialog repoUrl={sync.location ?? ""} onClose={() => setDialog(null)} onSaved={(go) => run(go)} />}
      {dialog === "serverKey" && sync.error?.kind === "hostKeyChanged" && (
        <ServerKeyChangedDialog
          expected={sync.error.expected}
          actual={sync.error.actual}
          onClose={() => setDialog(null)}
          onReplace={(fp) => {
            setDialog(null);
            void run(() => api.syncReplaceServerKey(fp));
          }}
        />
      )}
      {dialog === "disable" && (
        <ConfirmDialog
          title={t.disableTitle}
          confirm={t.disable}
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            try {
              await api.syncDisable();
              setSync(await api.syncStatus());
              toast({ text: t.disabled, tone: "success" });
            } catch (e) {
              toast({ text: errorText(e), tone: "error" });
            }
          }}
        />
      )}
    </section>
  );
}

/* ---------- turning it on ---------- */

/** Turns sync on in an empty folder or repository; where this person's vault already is,
 *  the same dialog connects this device to it with a code instead. */
function EnableDialog({ onClose }: { onClose: () => void }) {
  const { setSync, refreshPersonal, toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [storage, setStorageRaw] = useStorage();
  const [serverKey, setServerKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ field: "code" | "storage"; text: string } | null>(null);
  const [enabled, setEnabled] = useState<SyncInfo | null>(null);
  // The vault is there already: this device joins it.
  const [joining, setJoining] = useState(false);
  const [code, setCode] = useState("");

  const setStorage = (p: Partial<Storage>) => {
    setStorageRaw(p);
    setServerKey(null);
    setError(null);
    setJoining(false);
  };

  const storageError = (e: unknown) =>
    isCoreError(e) && e.kind === "folderNotEmpty" ? (storage.kind === "git" ? t.repoNotEmpty : t.folderNotEmpty) : isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? t.repoUnreachable : t.folderUnavailable) : errorText(e);

  const turnOn = async () => {
    try {
      setEnabled(await api.syncEnable(placeOf(storage), serverKey ?? undefined));
    } catch (e) {
      const fp = unknownServerKey(e);
      if (fp) setServerKey(fp);
      else if (isCoreError(e) && e.kind === "syncExists") setJoining(true);
      else setError({ field: "storage", text: storageError(e) });
    }
  };

  const join = async () => {
    try {
      const status = await api.syncJoin(placeOf(storage), code.trim(), serverKey ?? undefined);
      setSync(status);
      await refreshPersonal();
      toast({ text: t.enabled, tone: "success" });
      onClose();
    } catch (e) {
      const fp = unknownServerKey(e);
      if (fp) return setServerKey(fp);
      const onCode = isCoreError(e) && (e.kind === "invalidCode" || e.kind === "codeExpired");
      setError({ field: onCode ? "code" : "storage", text: storageError(e) });
    }
  };

  const ready = storageReady(storage) && (!joining || !!code.trim());
  const submit = async () => {
    if (busy || !ready) return;
    setBusy(true);
    setError(null);
    await (joining ? join() : turnOn());
    setBusy(false);
  };

  if (enabled) {
    return (
      <RecoveryDialog
        onClose={() => {
          setSync(enabled);
          toast({ text: t.enabled, tone: "success" });
          onClose();
        }}
      />
    );
  }

  const label = joining ? (serverKey ? t.trustAndConnect : t.connect) : serverKey ? t.trustAndEnable : t.enable;
  return (
    <Modal
      title={t.enableTitle}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!ready}>{label}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel={joining ? t.vaultFolder : t.emptyFolder} folderPlaceholder={"D:\Sync\detour"} />
      {joining && (
        <>
          <div className="banner info sync-found">
            <Link2 size={16} />
            <span>{storage.kind === "git" ? t.repoHasVault : t.folderHasVault}</span>
          </div>
          <Field label={t.code} error={error?.field === "code" ? error.text : null}>
            <input
              className={`input mono ${error?.field === "code" ? "invalid" : ""}`}
              value={code}
              onChange={(e) => {
                setCode(e.target.value);
                setError(null);
              }}
              placeholder="XXXX-XXXX-XXXX-XXXX"
              spellCheck={false}
              autoComplete="off"
              autoFocus
            />
          </Field>
        </>
      )}
      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}
      {error?.field === "storage" && <span className="field-error">{error.text}</span>}
    </Modal>
  );
}

/* ---------- adding a device ---------- */

/** What the phone scans: the code, and the repository as the phone reaches it (over HTTPS; a folder means nothing to it) or the drive folder. */
function joinLink(code: string, repo: string | null, drive: DriveFolderRef | null): string {
  const q = new URLSearchParams({ code });
  if (drive) {
    q.set("drive", JSON.stringify({ provider: drive.provider, id: drive.id, drive: drive.drive ?? undefined, label: drive.label, link: drive.link ?? undefined }));
    return `detour://join?${q}`;
  }
  const github = repo?.match(/^(?:git@github\.com:|ssh:\/\/git@github\.com\/)(.+?)(?:\.git)?\/?$/);
  const url = github ? `https://github.com/${github[1]}` : repo && /^https?:\/\//.test(repo) ? repo : null;
  if (url) q.set("repo", url);
  return `detour://join?${q}`;
}

function AddDeviceDialog({ repo, drive, onClose }: { repo: string | null; drive: DriveFolderRef | null; onClose: () => void }) {
  const { toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.syncAddDevice().then(setPairing).catch((e) => setError(errorText(e)));
  }, []);

  const qr = useMemo(() => (pairing ? renderSVG(joinLink(pairing.code, repo, drive), { ecc: "L", border: 2, whiteColor: "#fff", blackColor: "#000" }) : ""), [pairing, repo, drive]);
  const until = pairing ? new Date(pairing.expiresAt).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" }) : "";
  return (
    <Modal
      title={t.newDevice}
      width={440}
      onClose={onClose}
      onSubmit={onClose}
      footer={
        <>
          {pairing && (
            <Button type="button" icon={Copy} onClick={async () => toast((await copyText(pairing.code)) ? { text: t.codeCopied, tone: "success" } : { text: all.common.copyFailed, tone: "error" })}>
              {all.common.copy}
            </Button>
          )}
          <Button type="submit" variant="primary">{all.common.done}</Button>
        </>
      }
    >
      <div className="modal-text">{t.newDeviceHint}</div>
      {error ? (
        <span className="field-error">{error}</span>
      ) : (
        <div className={`pair-code mono ${pairing ? "" : "loading"}`}>{pairing?.code ?? "••••-••••-••••-••••"}</div>
      )}
      {pairing && <div className="pair-qr" dangerouslySetInnerHTML={{ __html: qr }} />}
      {pairing && <div className="muted small center">{t.validUntil(until)}</div>}
    </Modal>
  );
}

/* ---------- recovery key ---------- */

function RecoveryDialog({ onClose }: { onClose: () => void }) {
  const { toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [key, setKey] = useState("");
  useEffect(() => {
    api.syncRecoveryKey().then(setKey).catch((e) => toast({ text: errorText(e), tone: "error" }));
  }, [toast]);

  return (
    <Modal title={t.recoveryKey} width={540} onClose={onClose} onSubmit={onClose} footer={<Button type="submit" variant="primary">{all.common.done}</Button>}>
      <div className="modal-text">{t.recoveryHint}</div>
      <div className="recovery-box mono">{key || "…"}</div>
      <div className="row-buttons">
        <Button type="button" icon={Copy} disabled={!key} onClick={async () => { if (await copyText(key)) toast({ text: t.keyCopied, tone: "success" }); }}>{all.common.copy}</Button>
        <Button
          type="button"
          icon={Download}
          disabled={!key}
          onClick={async () => {
            const where = await api.saveTextFile("detour-recovery-personal.txt", t.recoveryFile(key));
            if (where) toast({ text: t.savedTo(where), tone: "success" });
          }}
        >
          {t.saveToFile}
        </Button>
      </div>
    </Modal>
  );
}

/* ---------- repository problems ---------- */

function TokenDialog({ repoUrl, onClose, onSaved }: { repoUrl: string; onClose: () => void; onSaved: (go: () => Promise<{ changed: boolean; status: SyncInfo }>) => Promise<void> }) {
  const all = useT();
  const t = all.sync;
  const [token, setToken] = useState("");
  const [useToken, setUseToken] = useState(false);
  const [githubReady, setGithubReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const viaGithub = !!githubRepo(repoUrl) && !useToken;
  const ready = viaGithub ? githubReady : !!token.trim();
  return (
    <Modal
      title={viaGithub ? all.teams.storage.login : t.tokenTitle}
      onClose={onClose}
      onSubmit={() => {
        if (!ready) return;
        onClose();
        void onSaved(viaGithub ? () => api.syncUseGithub() : () => api.syncSetToken(token.trim()));
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!ready}>{viaGithub ? all.teams.github.use : all.common.save}</Button>
        </>
      }
    >
      <RepoLoginFields url={repoUrl} token={token} setToken={setToken} error={error} setError={setError} useToken={useToken} setUseToken={setUseToken} onGithubReady={setGithubReady} />
    </Modal>
  );
}

function ServerKeyChangedDialog({ expected, actual, onClose, onReplace }: { expected: string; actual: string; onClose: () => void; onReplace: (fingerprint: string) => void }) {
  const all = useT();
  const t = all.sync;
  return (
    <Modal
      title={t.serverKeyChanged}
      onClose={onClose}
      footer={
        <>
          <Button type="button" variant="ghost" className="danger-text" onClick={() => onReplace(actual)}>{t.replaceKey}</Button>
          <Button type="button" variant="primary" autoFocus onClick={onClose}>{all.common.cancel}</Button>
        </>
      }
    >
      <dl className="hostkey-fp">
        <dt>{t.was}</dt>
        <dd className="mono">{expected}</dd>
        <dt>{t.now}</dt>
        <dd className="mono">{actual}</dd>
      </dl>
    </Modal>
  );
}
