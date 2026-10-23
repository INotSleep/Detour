import { useEffect, useState } from "react";
import { Copy, Download, FolderOpen, KeyRound, ShieldCheck, ShieldQuestion } from "lucide-react";
import { api, errorText, isCoreError, type GitLogin, type TeamView } from "../lib/api";
import { copyText, sleep, uid, useStore } from "../lib/store";
import type { Level, Role, Vault } from "../lib/types";
import { Button, Field, IconButton, Modal, PasswordInput, Segmented } from "./ui";

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

export function FolderField({ value, onChange, label = "Папка", placeholder = "D:\\Sync\\team-vault" }: { value: string; onChange: (v: string) => void; label?: string; placeholder?: string }) {
  return (
    <Field label={label}>
      <div className="input-group">
        <input className="input mono" value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} spellCheck={false} />
        {api.real && (
          <Button type="button" icon={FolderOpen} onClick={async () => { const p = await api.pickFolder(); if (p) onChange(p); }}>
            Выбрать
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
}

export function useStorage(): [Storage, (patch: Partial<Storage>) => void] {
  const { ws } = useStore();
  const firstKey = ws.keys.find((k) => k.vaultId === "personal")?.id ?? "agent";
  const [s, setS] = useState<Storage>({ kind: "folder", path: "", url: "", login: firstKey, token: "" });
  return [s, (patch) => setS((cur) => ({ ...cur, ...patch }))];
}

const isHttps = (url: string) => /^https?:\/\//i.test(url.trim());
export const gitLogin = (s: Storage): GitLogin =>
  isHttps(s.url)
    ? { url: s.url.trim(), auth: { type: "password" }, token: s.token.trim() }
    : { url: s.url.trim(), auth: s.login === "agent" ? { type: "agent" } : { type: "key", keyId: s.login } };
export const storageReady = (s: Storage) => (s.kind === "folder" ? !!s.path.trim() : !!s.url.trim() && (!isHttps(s.url) || !!s.token.trim()));

export function StorageFields({ value, set, folderLabel, folderPlaceholder }: { value: Storage; set: (p: Partial<Storage>) => void; folderLabel: string; folderPlaceholder?: string }) {
  const { ws, toast } = useStore();
  const keys = ws.keys.filter((k) => k.vaultId === "personal");
  const key = keys.find((k) => k.id === value.login);
  return (
    <>
      <Field label="Хранилище">
        <Segmented value={value.kind} onChange={(kind) => set({ kind })} options={[{ value: "folder", label: "Папка" }, { value: "git", label: "Git-репозиторий" }]} />
      </Field>
      {value.kind === "folder" ? (
        <FolderField value={value.path} onChange={(path) => set({ path })} label={folderLabel} placeholder={folderPlaceholder} />
      ) : (
        <>
          <Field label="Адрес репозитория">
            <input className="input mono" value={value.url} onChange={(e) => set({ url: e.target.value })} placeholder="git@github.com:team/vault.git" spellCheck={false} autoComplete="off" />
          </Field>
          {isHttps(value.url) ? (
            <Field label="Токен доступа">
              <PasswordInput value={value.token} onChange={(token) => set({ token })} placeholder="github_pat_…" />
            </Field>
          ) : (
          <Field label="Вход в репозиторий">
            <div className="input-group">
              <select className="input" value={value.login} onChange={(e) => set({ login: e.target.value })}>
                {keys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
                <option value="agent">ssh-agent</option>
              </select>
              {key && (
                <Button type="button" icon={Copy} onClick={async () => toast((await copyText(key.publicKey)) ? { text: `Публичный ключ «${key.name}» скопирован`, tone: "success" } : { text: "Не удалось скопировать", tone: "error" })}>
                  Публичный ключ
                </Button>
              )}
            </div>
          </Field>
          )}
        </>
      )}
    </>
  );
}

/** The server key of a repository seen for the first time. */
export function ServerKeyCard({ url, fingerprint }: { url: string; fingerprint: string }) {
  const host = url.replace(/^ssh:\/\//, "").replace(/^[^@]*@/, "").split(/[:/]/)[0];
  return (
    <div className="trust-card">
      <ShieldQuestion size={18} />
      <div>
        <div className="strong">Новый сервер {host}</div>
        <div className="fp mono">{fingerprint}</div>
        <div className="muted small">Сверьте отпечаток с тем, что публикует хостинг</div>
      </div>
    </div>
  );
}

/** A new server key from an error, if that is what it is. */
export const unknownServerKey = (e: unknown) => (isCoreError(e) && e.kind === "hostKeyUnknown" ? e.fingerprint : null);

/* ---------- my Detour ID ---------- */

export function MyIdDialog({ onClose }: { onClose: () => void }) {
  const { toast } = useStore();
  const me = useMyId();
  return (
    <Modal title="Мой Detour ID" width={480} onClose={onClose} footer={<Button variant="primary" icon={Copy} onClick={async () => { if (me && (await copyText(me.detourId))) toast({ text: "Detour ID скопирован", tone: "success" }); }}>Копировать</Button>}>
      <div className="id-box mono">{me?.detourId ?? "…"}</div>
      <div className="fp-row">
        <span className="muted small">Отпечаток</span>
        <span className="fp mono">{me?.fingerprint ?? "…"}</span>
      </div>
    </Modal>
  );
}

/* ---------- create, with a mandatory recovery key ---------- */

export function CreateTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
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
      toast({ text: `Команда «${name.trim()}» создана`, tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      const fp = unknownServerKey(e);
      if (fp) setServerKey(fp);
      else setError(isCoreError(e) && e.kind === "folderNotEmpty" ? (storage.kind === "git" ? "Репозиторий не пустой" : "Папка не пустая") : errorText(e));
    }
  };

  if (step === "details") {
    return (
      <Modal
        title="Новая команда"
        width={520}
        onClose={onClose}
        onSubmit={next}
        footer={
          <>
            <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
            <Button type="submit" variant="primary" disabled={!detailsOk}>Далее</Button>
          </>
        }
      >
        <Field label="Название">
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Infra" autoFocus />
        </Field>
        <StorageFields value={storage} set={(p) => { setStorage(p); setServerKey(null); }} folderLabel="Пустая папка для хранилища" />
        <Field label="Ваше имя в команде">
          <input className="input" value={myName} onChange={(e) => setMyName(e.target.value)} placeholder="Владислав" />
        </Field>
      </Modal>
    );
  }

  return (
    <Modal
      title="Ключ восстановления"
      width={540}
      onClose={onClose}
      onSubmit={create}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => setStep("details")}>Назад</Button>
          <Button type="submit" variant="primary" disabled={!confirmed} loading={busy}>{serverKey ? "Доверять и создать" : "Создать команду"}</Button>
        </>
      }
    >
      <div className="modal-text">Без него не восстановить команду, если владельцы потеряют устройства.</div>
      <div className="recovery-box mono">{recovery}</div>
      <div className="row-buttons">
        <Button type="button" icon={Copy} onClick={async () => { if (await copyText(recovery)) { toast({ text: "Ключ скопирован", tone: "success" }); } }}>Копировать</Button>
        <Button
          type="button"
          icon={Download}
          onClick={async () => {
            const where = await api.saveTextFile(`detour-recovery-${name.trim().toLowerCase().replace(/\s+/g, "-") || "team"}.txt`, `${name.trim()}\n${recovery}\n`);
            if (where) toast({ text: `Сохранено: ${where}`, tone: "success" });
          }}
        >
          Сохранить в файл
        </Button>
      </div>
      <Field label="Последние 4 символа ключа" error={confirm.length >= 4 && !confirmed ? "Не совпадает" : error}>
        <input className={`input mono confirm-input ${confirm.length >= 4 && !confirmed ? "invalid" : ""}`} value={confirm} onChange={(e) => setConfirm(e.target.value.slice(0, 4))} placeholder="••••" autoComplete="off" spellCheck={false} />
      </Field>
      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}
    </Modal>
  );
}

/* ---------- join (or recover) ---------- */

export function JoinTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
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
    toast({ text: view.joined ? `Команда «${view.name}» подключена` : "Хранилище подключено. Ждём, пока админ добавит ваш Detour ID", tone: "success" });
    onClose();
  };

  const submit = async () => {
    if (busy || !storageReady(storage)) return;
    if (!api.real) {
      toast({ text: "Подключение работает в приложении Detour", tone: "error" });
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
      else setError(isCoreError(e) && e.kind === "io" ? (storage.kind === "git" ? "Нет связи с репозиторием" : "Папка недоступна") : errorText(e));
    }
  };

  const title = mode === "recover" ? "Восстановить доступ" : "Подключить команду";
  const action = serverKey ? "Доверять серверу" : mode === "recover" ? "Восстановить" : info ? "Доверять и подключить" : "Далее";

  return (
    <Modal
      title={title}
      width={540}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" className="footer-left" onClick={() => { setMode(mode === "join" ? "recover" : "join"); setInfo(null); setClone(null); setError(null); }}>
            {mode === "join" ? "У меня ключ восстановления" : "Обычное подключение"}
          </Button>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!storageReady(storage) || (mode === "recover" && !recoveryKey.trim())}>{action}</Button>
        </>
      }
    >
      <StorageFields value={storage} set={setStorage} folderLabel="Папка с хранилищем команды" />

      {serverKey && <ServerKeyCard url={storage.url} fingerprint={serverKey} />}

      {mode === "join" && info && (
        <div className="trust-card">
          <ShieldCheck size={18} />
          <div>
            <div className="strong">Создатель хранилища</div>
            <div className="fp mono">{info.rootFingerprint}</div>
            <div className="muted small">Сверьте отпечаток с создателем команды</div>
          </div>
        </div>
      )}

      {mode === "recover" && (
        <>
          <Field label="Ключ восстановления">
            <textarea className="input mono textarea" rows={3} value={recoveryKey} onChange={(e) => setRecoveryKey(e.target.value)} placeholder="DETOUR-RECOVERY-…" spellCheck={false} />
          </Field>
          <Field label="Ваше имя в команде">
            <input className="input" value={myName} onChange={(e) => setMyName(e.target.value)} />
          </Field>
        </>
      )}

      {error && <span className="field-error">{error}</span>}
      {mode === "join" && me && (
        <div className="muted small">Ваш отпечаток: <span className="mono">{me.fingerprint}</span></div>
      )}
    </Modal>
  );
}

/* ---------- add a member by Detour ID ---------- */

export function AddMemberDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
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
    const t = setTimeout(async () => {
      if (!api.real) {
        if (/^detour1[a-z2-7]{100,}$/i.test(id)) setFingerprint(`${demoGroup()}-${demoGroup()}-${demoGroup()}-${demoGroup()}`);
        else setIdError("Неверный Detour ID");
        return;
      }
      try {
        setFingerprint(await api.idInspect(id));
      } catch {
        setIdError("Неверный Detour ID");
      }
    }, 250);
    return () => clearTimeout(t);
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
          activity: [...w.activity, { vaultId: vault.id, authorName: "Вы", isYou: true, at: Date.now(), kind: "memberAdded", member: name.trim() }],
        }));
      }
      toast({ text: `${name.trim()} добавлен в команду`, tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      if (isCoreError(e) && e.kind === "alreadyMember") setIdError("Уже в команде");
      else toast({ text: errorText(e), tone: "error" });
    }
  };

  return (
    <Modal
      title={`Добавить в «${vault.name}»`}
      width={500}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!valid} loading={busy}>Добавить</Button>
        </>
      }
    >
      <Field label="Detour ID участника" error={idError}>
        <textarea className={`input mono textarea id-input ${idError ? "invalid" : ""}`} rows={3} value={detourId} onChange={(e) => setDetourId(e.target.value)} placeholder="detour1…" spellCheck={false} autoFocus />
      </Field>
      {fingerprint && (
        <div className="fp-row">
          <span className="muted small">Отпечаток</span>
          <span className="fp mono">{fingerprint}</span>
        </div>
      )}
      <div className="row">
        <Field label="Имя">
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Anna" />
        </Field>
        <Field label="Роль">
          <select className="input" value={role} onChange={(e) => setRole(e.target.value as Role)}>
            {iAmOwner && <option value="owner">Владелец</option>}
            {iAmOwner && <option value="admin">Админ</option>}
            <option value="member">Участник</option>
          </select>
        </Field>
      </div>
      <Field label="Доступ ко всем записям">
        <Segmented
          value={access}
          onChange={setAccess}
          options={[
            { value: "none", label: "Нет" },
            { value: "view", label: "Видит" },
            { value: "secret", label: "Секрет" },
            { value: "edit", label: "Изменяет" },
          ]}
        />
      </Field>
    </Modal>
  );
}

export function CopyIdButton() {
  const { toast } = useStore();
  const me = useMyId();
  return <IconButton icon={KeyRound} label="Копировать мой Detour ID" onClick={async () => { if (me && (await copyText(me.detourId))) toast({ text: "Detour ID скопирован", tone: "success" }); }} />;
}
