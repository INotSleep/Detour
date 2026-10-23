import { useEffect, useState } from "react";
import { Copy, Download, FolderOpen, KeyRound, ShieldCheck } from "lucide-react";
import { api, errorText, isCoreError, type TeamView } from "../lib/api";
import { copyText, sleep, uid, useStore } from "../lib/store";
import type { Role, Vault } from "../lib/types";
import { Button, Checkbox, Field, IconButton, Modal } from "./ui";

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

function FolderField({ value, onChange, label = "Папка" }: { value: string; onChange: (v: string) => void; label?: string }) {
  return (
    <Field label={label}>
      <div className="input-group">
        <input className="input mono" value={value} onChange={(e) => onChange(e.target.value)} placeholder={"D:\\Sync\\team-vault"} spellCheck={false} />
        {api.real && (
          <Button type="button" icon={FolderOpen} onClick={async () => { const p = await api.pickFolder(); if (p) onChange(p); }}>
            Выбрать
          </Button>
        )}
      </div>
    </Field>
  );
}

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
  const [path, setPath] = useState("");
  const [myName, setMyName] = useState("");
  const [recovery, setRecovery] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const lastFour = recovery.replace(/[^A-Z0-9]/gi, "").slice(-4).toUpperCase();
  const confirmed = confirm.replace(/[^A-Z0-9]/gi, "").toUpperCase() === lastFour;
  const detailsOk = name.trim() && path.trim() && myName.trim();

  const next = async () => {
    if (!detailsOk) return;
    setRecovery(api.real ? await api.recoveryKeyNew() : demoRecoveryKey());
    setStep("recovery");
  };

  const create = async () => {
    if (!confirmed || busy) return;
    setBusy(true);
    try {
      if (api.real) {
        const view: TeamView = await api.teamCreate(path.trim(), name.trim(), myName.trim(), recovery);
        applyTeamView(view);
        setVault(view.id);
      } else {
        await sleep(500);
        const id = uid("v");
        const vault: Vault = { id, name: name.trim(), kind: "team", sync: { type: "folder", path: path.trim() }, syncStatus: "synced", lastSync: Date.now(), conflicts: [], joined: true, myRole: "owner", canReveal: true, problems: 0 };
        update((w) => ({
          ...w,
          vaults: [...w.vaults, vault],
          members: [
            ...w.members,
            { id: uid("m"), vaultId: id, name: myName.trim(), role: "owner", canReveal: true, recovery: false, fingerprint: DEMO_ID.fingerprint, isYou: true },
            { id: uid("m"), vaultId: id, name: "Recovery key", role: "owner", canReveal: true, recovery: true, fingerprint: demoGroup() + "-" + demoGroup(), isYou: false },
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
      setError(errorText(e));
    }
  };

  if (step === "details") {
    return (
      <Modal
        title="Новая команда"
        width={500}
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
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Infra" />
        </Field>
        <FolderField value={path} onChange={setPath} label="Пустая папка для хранилища" />
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
          <Button type="submit" variant="primary" disabled={!confirmed} loading={busy}>Создать команду</Button>
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
            if (where) {
             
              toast({ text: `Сохранено: ${where}`, tone: "success" });
            }
          }}
        >
          Сохранить в файл
        </Button>
      </div>
      <Field label="Последние 4 символа ключа" error={confirm.length >= 4 && !confirmed ? "Не совпадает" : error}>
        <input className={`input mono confirm-input ${confirm.length >= 4 && !confirmed ? "invalid" : ""}`} value={confirm} onChange={(e) => setConfirm(e.target.value.slice(0, 4))} placeholder="••••" autoComplete="off" spellCheck={false} />
      </Field>
    </Modal>
  );
}

/* ---------- join (or recover) ---------- */

export function JoinTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
  const { setVault, toast, applyTeamView } = useStore();
  const me = useMyId();
  const [mode, setMode] = useState<"join" | "recover">("join");
  const [path, setPath] = useState("");
  const [info, setInfo] = useState<{ root: string; rootFingerprint: string } | null>(null);
  const [recoveryKey, setRecoveryKey] = useState("");
  const [myName, setMyName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const finish = (view: TeamView) => {
    applyTeamView(view);
    setVault(view.id);
    onDone?.();
    toast({ text: view.joined ? `Команда «${view.name}» подключена` : "Хранилище подключено. Ждём, пока админ добавит ваш Detour ID", tone: "success" });
    onClose();
  };

  const submit = async () => {
    if (busy || !path.trim()) return;
    if (!api.real) {
      toast({ text: "Подключение работает в приложении Detour", tone: "error" });
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (mode === "recover") return finish(await api.teamRecover(path.trim(), recoveryKey, myName.trim() || "Owner"));
      if (!info) {
        setInfo(await api.teamInspect(path.trim()));
        setBusy(false);
        return;
      }
      finish(await api.teamJoin(path.trim(), info.root));
    } catch (e) {
      setBusy(false);
      setError(isCoreError(e) && e.kind === "io" ? "Папка недоступна" : errorText(e));
    }
  };

  const title = mode === "recover" ? "Восстановить доступ" : "Подключить команду";
  const action = mode === "recover" ? "Восстановить" : info ? "Доверять и подключить" : "Далее";

  return (
    <Modal
      title={title}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" className="footer-left" onClick={() => { setMode(mode === "join" ? "recover" : "join"); setInfo(null); setError(null); }}>
            {mode === "join" ? "У меня ключ восстановления" : "Обычное подключение"}
          </Button>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!path.trim() || (mode === "recover" && !recoveryKey.trim())}>{action}</Button>
        </>
      }
    >
      <FolderField value={path} onChange={(v) => { setPath(v); setInfo(null); setError(null); }} label="Папка с хранилищем команды" />

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
  const [canReveal, setCanReveal] = useState(false);
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
        applyTeamView(await api.teamMemberAdd(vault.id, detourId.trim(), name.trim(), role, canReveal));
      } else {
        await sleep(400);
        update((w) => ({
          ...w,
          members: [...w.members, { id: uid("m"), vaultId: vault.id, name: name.trim(), role, canReveal, recovery: false, fingerprint: fingerprint!, isYou: false }],
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
            <option value="readOnly">Только чтение</option>
          </select>
        </Field>
      </div>
      <Checkbox checked={canReveal} onChange={setCanReveal}>Видит секреты: пароли и приватные ключи</Checkbox>
    </Modal>
  );
}

export function CopyIdButton() {
  const { toast } = useStore();
  const me = useMyId();
  return <IconButton icon={KeyRound} label="Копировать мой Detour ID" onClick={async () => { if (me && (await copyText(me.detourId))) toast({ text: "Detour ID скопирован", tone: "success" }); }} />;
}
