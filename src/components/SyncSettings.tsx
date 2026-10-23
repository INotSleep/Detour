import { useEffect, useState } from "react";
import { ChevronRight, Copy, Download, FolderSync, GitBranch, Laptop, RotateCw, ShieldAlert } from "lucide-react";
import { api, errorText, isCoreError, type CoreError, type Pairing, type SyncInfo, type SyncPlace } from "../lib/api";
import { locale, tr, useT } from "../i18n";
import { copyText, formatAgo, useStore } from "../lib/store";
import { Button, ConfirmDialog, Field, Modal, PasswordInput } from "./ui";
import { ServerKeyCard, StorageFields, gitLogin, storageReady, unknownServerKey, useStorage, type Storage } from "./TeamDialogs";
import { problemText, remoteErrorText } from "./TeamsView";

const placeOf = (s: Storage): SyncPlace => (s.kind === "folder" ? { path: s.path.trim() } : gitLogin(s));

/** What went wrong with the last sync, in words. */
export function syncErrorText(s: SyncInfo): string {
  const e = s.error;
  if (!e) return "";
  if (!s.git && (e.kind === "unavailable" || e.kind === "io")) return tr().sync.folderUnavailable;
  return remoteErrorText(e);
}

const tokenRejected = (e: CoreError | null) => e?.kind === "git" && e.message.includes("token rejected");

/** Settings: syncing the personal vault between this person's devices. */
export function SyncSettings() {
  const { sync, setSync, refreshPersonal, toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [dialog, setDialog] = useState<"enable" | "join" | "device" | "recovery" | "disable" | "token" | "serverKey" | null>(null);
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
            <Button size="sm" onClick={() => setDialog("join")}>{all.common.connect}</Button>
            <Button size="sm" variant="primary" onClick={() => setDialog("enable")}>{t.enable}</Button>
          </div>
        </div>
        {dialog === "enable" && <EnableDialog onClose={() => setDialog(null)} />}
        {dialog === "join" && <JoinDialog onClose={() => setDialog(null)} />}
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
        <div className="item-icon">{sync.git ? <GitBranch size={15} /> : <FolderSync size={15} />}</div>
        <div className="sync-main">
          <span className="mono sync-location" title={sync.location ?? ""}>{sync.location}</span>
          <span className={`small ${failed ? "err-text" : sync.pending ? "warn-text" : "muted"}`}>{status}</span>
        </div>
        {keyChanged ? (
          <Button size="sm" onClick={() => setDialog("serverKey")}>{t.replaceServerKey}</Button>
        ) : tokenRejected(sync.error) ? (
          <Button size="sm" onClick={() => setDialog("token")}>{t.newToken}</Button>
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

      {dialog === "device" && <AddDeviceDialog onClose={() => setDialog(null)} />}
      {dialog === "recovery" && <RecoveryDialog onClose={() => setDialog(null)} />}
      {dialog === "token" && <TokenDialog onClose={() => setDialog(null)} onSaved={(go) => run(go)} />}
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

function EnableDialog({ onClose }: { onClose: () => void }) {
  const { setSync, toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [storage, setStorageRaw] = useStorage();
  const [serverKey, setServerKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [enabled, setEnabled] = useState<SyncInfo | null>(null);

  const setStorage = (p: Partial<Storage>) => {
    setStorageRaw(p);
    setServerKey(null);
    setError(null);
  };

  const submit = async () => {
    if (busy || !storageReady(storage)) return;
    setBusy(true);
    setError(null);
    try {
      setEnabled(await api.syncEnable(placeOf(storage), serverKey ?? undefined));
    } catch (e) {
      const fp = unknownServerKey(e);
      if (fp) setServerKey(fp);
      else setError(isCoreError(e) && e.kind === "folderNotEmpty" ? (storage.kind === "git" ? t.repoNotEmpty : t.folderNotEmpty) : isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? t.repoUnreachable : t.folderUnavailable) : errorText(e));
    } finally {
      setBusy(false);
    }
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

  return (
    <Modal
      title={t.enableTitle}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!storageReady(storage)}>{serverKey ? t.trustAndEnable : t.enable}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel={t.emptyFolder} folderPlaceholder={"D:\\Sync\\detour"} />
      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}
      {error && <span className="field-error">{error}</span>}
    </Modal>
  );
}

/* ---------- joining from a new device ---------- */

function JoinDialog({ onClose }: { onClose: () => void }) {
  const { setSync, refreshPersonal, toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [storage, setStorageRaw] = useStorage();
  const [code, setCode] = useState("");
  const [serverKey, setServerKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ field: "code" | "storage"; text: string } | null>(null);

  const setStorage = (p: Partial<Storage>) => {
    setStorageRaw(p);
    setServerKey(null);
    setError(null);
  };

  const submit = async () => {
    if (busy || !storageReady(storage) || !code.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const status = await api.syncJoin(placeOf(storage), code.trim(), serverKey ?? undefined);
      setSync(status);
      await refreshPersonal();
      toast({ text: t.enabled, tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      const fp = unknownServerKey(e);
      if (fp) return setServerKey(fp);
      const onCode = isCoreError(e) && (e.kind === "invalidCode" || e.kind === "codeExpired");
      const text = isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? t.repoUnreachable : t.folderUnavailable) : errorText(e);
      setError({ field: onCode ? "code" : "storage", text });
    }
  };

  return (
    <Modal
      title={t.joinTitle}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!storageReady(storage) || !code.trim()}>{serverKey ? t.trustAndConnect : t.connect}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel={t.vaultFolder} folderPlaceholder={"D:\\Sync\\detour"} />
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
        />
      </Field>
      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}
      {error?.field === "storage" && <span className="field-error">{error.text}</span>}
    </Modal>
  );
}

/* ---------- adding a device ---------- */

function AddDeviceDialog({ onClose }: { onClose: () => void }) {
  const { toast } = useStore();
  const all = useT();
  const t = all.sync;
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.syncAddDevice().then(setPairing).catch((e) => setError(errorText(e)));
  }, []);

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

function TokenDialog({ onClose, onSaved }: { onClose: () => void; onSaved: (go: () => Promise<{ changed: boolean; status: SyncInfo }>) => Promise<void> }) {
  const all = useT();
  const t = all.sync;
  const [token, setToken] = useState("");
  return (
    <Modal
      title={t.tokenTitle}
      onClose={onClose}
      onSubmit={() => {
        if (!token.trim()) return;
        onClose();
        void onSaved(() => api.syncSetToken(token.trim()));
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{all.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!token.trim()}>{all.common.save}</Button>
        </>
      }
    >
      <Field label={t.newToken}>
        <PasswordInput value={token} onChange={setToken} autoFocus />
      </Field>
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
