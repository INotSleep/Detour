import { useEffect, useState } from "react";
import { ChevronRight, Copy, Download, FolderSync, GitBranch, Laptop, RotateCw, ShieldAlert } from "lucide-react";
import { api, errorText, isCoreError, type CoreError, type Pairing, type SyncInfo, type SyncPlace } from "../lib/api";
import { copyText, formatAgo, plural, useStore } from "../lib/store";
import { Button, ConfirmDialog, Field, Modal, PasswordInput } from "./ui";
import { ServerKeyCard, StorageFields, gitLogin, storageReady, unknownServerKey, useStorage, type Storage } from "./TeamDialogs";
import { problemText, remoteErrorText } from "./TeamsView";

const placeOf = (s: Storage): SyncPlace => (s.kind === "folder" ? { path: s.path.trim() } : gitLogin(s));

/** What went wrong with the last sync, in words. */
export function syncErrorText(s: SyncInfo): string {
  const e = s.error;
  if (!e) return "";
  if (!s.git && (e.kind === "unavailable" || e.kind === "io")) return "Папка недоступна";
  return remoteErrorText(e);
}

const tokenRejected = (e: CoreError | null) => e?.kind === "git" && e.message.includes("token rejected");

/** Settings: syncing the personal vault between this person's devices. */
export function SyncSettings() {
  const { sync, setSync, refreshPersonal, toast } = useStore();
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
        <h3 className="section-title">Синхронизация</h3>
        <div className="setting">
          <span>Между своими устройствами</span>
          <div className="row-buttons">
            <Button size="sm" onClick={() => setDialog("join")}>Подключиться</Button>
            <Button size="sm" variant="primary" onClick={() => setDialog("enable")}>Включить</Button>
          </div>
        </div>
        {dialog === "enable" && <EnableDialog onClose={() => setDialog(null)} />}
        {dialog === "join" && <JoinDialog onClose={() => setDialog(null)} />}
      </section>
    );
  }

  const failed = !!sync.error;
  const keyChanged = sync.error?.kind === "hostKeyChanged";
  const status = failed ? syncErrorText(sync) : syncing ? "Синхронизация…" : sync.pending ? "Не отправлено" : sync.syncedAt ? `Синхронизировано ${formatAgo(sync.syncedAt)}` : "Ещё не синхронизировано";
  const problems = sync.problems;

  return (
    <section className="settings-group">
      <h3 className="section-title">Синхронизация</h3>
      <div className="sync-card">
        <div className="item-icon">{sync.git ? <GitBranch size={15} /> : <FolderSync size={15} />}</div>
        <div className="sync-main">
          <span className="mono sync-location" title={sync.location ?? ""}>{sync.location}</span>
          <span className={`small ${failed ? "err-text" : sync.pending ? "warn-text" : "muted"}`}>{status}</span>
        </div>
        {keyChanged ? (
          <Button size="sm" onClick={() => setDialog("serverKey")}>Заменить ключ сервера</Button>
        ) : tokenRejected(sync.error) ? (
          <Button size="sm" onClick={() => setDialog("token")}>Новый токен</Button>
        ) : (
          <Button size="sm" icon={RotateCw} loading={syncing} onClick={() => void run(() => api.syncNow())}>Синхронизировать</Button>
        )}
      </div>

      {problems.length > 0 && (
        <div className="banner warn problems">
          <button className="problems-head" onClick={() => setShowProblems((s) => !s)} aria-expanded={showProblems}>
            <ShieldAlert size={16} />
            <span className="grow">{problems.length} {plural(problems.length, "проблема", "проблемы", "проблем")} с хранилищем</span>
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
        <span>Другое устройство</span>
        <Button size="sm" icon={Laptop} onClick={() => setDialog("device")}>Добавить</Button>
      </div>
      <div className="setting">
        <span>Ключ восстановления</span>
        <Button size="sm" onClick={() => setDialog("recovery")}>Показать</Button>
      </div>
      <div className="setting">
        <span>Синхронизация на этом устройстве</span>
        <Button size="sm" variant="ghost" className="danger-text" onClick={() => setDialog("disable")}>Отключить</Button>
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
          title="Отключить синхронизацию?"
          confirm="Отключить"
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            try {
              await api.syncDisable();
              setSync(await api.syncStatus());
              toast({ text: "Синхронизация отключена", tone: "success" });
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
      else setError(isCoreError(e) && e.kind === "folderNotEmpty" ? (storage.kind === "git" ? "Репозиторий не пустой" : "Папка не пустая") : isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? "Нет связи с репозиторием" : "Папка недоступна") : errorText(e));
    } finally {
      setBusy(false);
    }
  };

  if (enabled) {
    return (
      <RecoveryDialog
        onClose={() => {
          setSync(enabled);
          toast({ text: "Синхронизация включена", tone: "success" });
          onClose();
        }}
      />
    );
  }

  return (
    <Modal
      title="Синхронизация"
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!storageReady(storage)}>{serverKey ? "Доверять и включить" : "Включить"}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel="Пустая папка" folderPlaceholder={"D:\\Sync\\detour"} />
      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}
      {error && <span className="field-error">{error}</span>}
    </Modal>
  );
}

/* ---------- joining from a new device ---------- */

function JoinDialog({ onClose }: { onClose: () => void }) {
  const { setSync, refreshPersonal, toast } = useStore();
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
      toast({ text: "Синхронизация включена", tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      const fp = unknownServerKey(e);
      if (fp) return setServerKey(fp);
      const onCode = isCoreError(e) && (e.kind === "invalidCode" || e.kind === "codeExpired");
      const text = isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? "Нет связи с репозиторием" : "Папка недоступна") : errorText(e);
      setError({ field: onCode ? "code" : "storage", text });
    }
  };

  return (
    <Modal
      title="Подключение к своему vault"
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!storageReady(storage) || !code.trim()}>{serverKey ? "Доверять и подключить" : "Подключить"}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel="Папка с хранилищем" folderPlaceholder={"D:\\Sync\\detour"} />
      <Field label="Код с другого устройства или ключ восстановления" error={error?.field === "code" ? error.text : null}>
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
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.syncAddDevice().then(setPairing).catch((e) => setError(errorText(e)));
  }, []);

  const until = pairing ? new Date(pairing.expiresAt).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" }) : "";
  return (
    <Modal
      title="Новое устройство"
      width={440}
      onClose={onClose}
      onSubmit={onClose}
      footer={
        <>
          {pairing && (
            <Button type="button" icon={Copy} onClick={async () => toast((await copyText(pairing.code)) ? { text: "Код скопирован", tone: "success" } : { text: "Не удалось скопировать", tone: "error" })}>
              Копировать
            </Button>
          )}
          <Button type="submit" variant="primary">Готово</Button>
        </>
      }
    >
      <div className="modal-text">На новом устройстве: Настройки, Синхронизация, Подключиться. То же хранилище и этот код.</div>
      {error ? (
        <span className="field-error">{error}</span>
      ) : (
        <div className={`pair-code mono ${pairing ? "" : "loading"}`}>{pairing?.code ?? "••••-••••-••••-••••"}</div>
      )}
      {pairing && <div className="muted small center">Действует до {until}</div>}
    </Modal>
  );
}

/* ---------- recovery key ---------- */

function RecoveryDialog({ onClose }: { onClose: () => void }) {
  const { toast } = useStore();
  const [key, setKey] = useState("");
  useEffect(() => {
    api.syncRecoveryKey().then(setKey).catch((e) => toast({ text: errorText(e), tone: "error" }));
  }, [toast]);

  return (
    <Modal title="Ключ восстановления" width={540} onClose={onClose} onSubmit={onClose} footer={<Button type="submit" variant="primary">Готово</Button>}>
      <div className="modal-text">Сохраните его: с ним vault открывается без других устройств.</div>
      <div className="recovery-box mono">{key || "…"}</div>
      <div className="row-buttons">
        <Button type="button" icon={Copy} disabled={!key} onClick={async () => { if (await copyText(key)) toast({ text: "Ключ скопирован", tone: "success" }); }}>Копировать</Button>
        <Button
          type="button"
          icon={Download}
          disabled={!key}
          onClick={async () => {
            const where = await api.saveTextFile("detour-recovery-personal.txt", `Detour: личный vault\n${key}\n`);
            if (where) toast({ text: `Сохранено: ${where}`, tone: "success" });
          }}
        >
          Сохранить в файл
        </Button>
      </div>
    </Modal>
  );
}

/* ---------- repository problems ---------- */

function TokenDialog({ onClose, onSaved }: { onClose: () => void; onSaved: (go: () => Promise<{ changed: boolean; status: SyncInfo }>) => Promise<void> }) {
  const [token, setToken] = useState("");
  return (
    <Modal
      title="Токен доступа к репозиторию"
      onClose={onClose}
      onSubmit={() => {
        if (!token.trim()) return;
        onClose();
        void onSaved(() => api.syncSetToken(token.trim()));
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!token.trim()}>Сохранить</Button>
        </>
      }
    >
      <Field label="Новый токен">
        <PasswordInput value={token} onChange={setToken} autoFocus />
      </Field>
    </Modal>
  );
}

function ServerKeyChangedDialog({ expected, actual, onClose, onReplace }: { expected: string; actual: string; onClose: () => void; onReplace: (fingerprint: string) => void }) {
  return (
    <Modal
      title="Ключ сервера изменился"
      onClose={onClose}
      footer={
        <>
          <Button type="button" variant="ghost" className="danger-text" onClick={() => onReplace(actual)}>Заменить ключ</Button>
          <Button type="button" variant="primary" autoFocus onClick={onClose}>Отменить</Button>
        </>
      }
    >
      <dl className="hostkey-fp">
        <dt>Был</dt>
        <dd className="mono">{expected}</dd>
        <dt>Сейчас</dt>
        <dd className="mono">{actual}</dd>
      </dl>
    </Modal>
  );
}
