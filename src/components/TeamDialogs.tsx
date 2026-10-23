import { useState } from "react";
import { Copy } from "lucide-react";
import { copyText, sleep, uid, useStore } from "../lib/store";
import type { Member, Role, Vault } from "../lib/types";
import { Button, Field, IconButton, Modal, Segmented } from "./ui";

export function CreateTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
  const { update, setVault, toast } = useStore();
  const [name, setName] = useState("");
  const [kind, setKind] = useState<"git" | "folder">("git");
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);

  const valid = name.trim() && target.trim();

  const submit = async () => {
    if (!valid || busy) return;
    setBusy(true);
    await sleep(900);
    const id = uid("v");
    const vault: Vault = {
      id,
      name: name.trim(),
      kind: "team",
      sync: kind === "git" ? { type: "git", remote: target.trim(), branch: "main" } : { type: "folder", path: target.trim() },
      syncStatus: "synced",
      lastSync: "только что",
      conflicts: [],
    };
    const me: Member = { id: uid("m"), vaultId: id, name: "Владислав", email: "vlad@acme.dev", role: "owner", fingerprint: "age1q8m…r4xz", status: "active", isYou: true };
    update((ws) => ({ ...ws, vaults: [...ws.vaults, vault], members: [...ws.members, me] }));
    setVault(id);
    onDone?.();
    toast({ text: `Команда «${vault.name}» создана`, tone: "success" });
    onClose();
  };

  return (
    <Modal
      title="Новая команда"
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!valid} loading={busy}>Создать</Button>
        </>
      }
    >
      <Field label="Название">
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Infra" />
      </Field>
      <Field label="Хранилище">
        <Segmented value={kind} onChange={(v) => { setKind(v); setTarget(""); }} options={[{ value: "git", label: "Git-репозиторий" }, { value: "folder", label: "Папка" }]} />
      </Field>
      <Field label={kind === "git" ? "Адрес репозитория" : "Путь к папке"}>
        <input className="input mono" value={target} onChange={(e) => setTarget(e.target.value)} placeholder={kind === "git" ? "git@github.com:org/vault.git" : "~/Syncthing/team"} />
      </Field>
    </Modal>
  );
}

export function JoinTeamDialog({ onClose, onDone }: { onClose: () => void; onDone?: () => void }) {
  const { update, setVault, toast } = useStore();
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!code.trim() || busy) return;
    const m = code.trim().match(/^detour-invite:v1:([^:]+):\S+$/);
    if (!m) {
      setError("Неверный код приглашения");
      return;
    }
    setBusy(true);
    await sleep(1100);
    const id = uid("v");
    const name = decodeURIComponent(m[1]!);
    update((ws) => ({
      ...ws,
      vaults: [...ws.vaults, { id, name, kind: "team", sync: { type: "git", remote: `git@github.com:${name.toLowerCase()}/detour-vault.git`, branch: "main" }, syncStatus: "synced", lastSync: "только что", conflicts: [] }],
      members: [...ws.members, { id: uid("m"), vaultId: id, name: "Владислав", email: "vlad@acme.dev", role: "member", fingerprint: "age1q8m…r4xz", status: "active", isYou: true }],
    }));
    setVault(id);
    onDone?.();
    toast({ text: `Вы вступили в «${name}»`, tone: "success" });
    onClose();
  };

  return (
    <Modal
      title="Вступить в команду"
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!code.trim()} loading={busy}>Вступить</Button>
        </>
      }
    >
      <Field label="Код приглашения" error={error}>
        <input
          className={`input mono ${error ? "invalid" : ""}`}
          value={code}
          onChange={(e) => {
            setCode(e.target.value);
            setError(null);
          }}
          placeholder="detour-invite:v1:…"
        />
      </Field>
    </Modal>
  );
}

export function InviteDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
  const { update, toast } = useStore();
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [busy, setBusy] = useState(false);
  const code = `detour-invite:v1:${encodeURIComponent(vault.name)}:3f9ac1e8b27d40c71e`;

  const emailValid = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());

  const submit = async () => {
    if (!emailValid || busy) return;
    setBusy(true);
    await sleep(500);
    const e = email.trim();
    update((ws) => ({
      ...ws,
      members: [...ws.members, { id: uid("m"), vaultId: vault.id, name: e, email: e, role, fingerprint: "", status: "invited" }],
      activity: [{ id: uid("a"), vaultId: vault.id, memberId: ws.members.find((m) => m.vaultId === vault.id && m.isYou)!.id, action: "пригласили", target: e, when: "только что" }, ...ws.activity],
    }));
    const copied = await copyText(code);
    toast({ text: copied ? "Приглашение создано, код скопирован" : "Приглашение создано", tone: "success" });
    onClose();
  };

  return (
    <Modal
      title={`Пригласить в «${vault.name}»`}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!emailValid} loading={busy}>Пригласить и скопировать код</Button>
        </>
      }
    >
      <Field label="Email">
        <input className="input" type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@company.com" />
      </Field>
      <Field label="Роль">
        <select className="input" value={role} onChange={(e) => setRole(e.target.value as Role)}>
          <option value="admin">Админ</option>
          <option value="member">Участник</option>
          <option value="readOnly">Только чтение</option>
        </select>
      </Field>
      <Field label="Код приглашения">
        <div className="input-group">
          <input className="input mono" readOnly value={code} onFocus={(e) => e.target.select()} />
          <IconButton
            type="button"
            icon={Copy}
            label="Копировать"
            onClick={async () => toast(await copyText(code) ? { text: "Код скопирован", tone: "success" } : { text: "Не удалось скопировать", tone: "error" })}
          />
        </div>
      </Field>
    </Modal>
  );
}
