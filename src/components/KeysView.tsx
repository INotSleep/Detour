import { useRef, useState } from "react";
import { Copy, Download, Ellipsis, FileUp, KeyRound, Lock, Plus, Trash2, Upload } from "lucide-react";
import { api } from "../lib/api";
import { fakeKeyMaterial, parsePrivateKey, suggestKeyName } from "../lib/keys";
import { copyText, plural, sleep, uid, useStore } from "../lib/store";
import type { SshKey } from "../lib/types";
import { Button, Checkbox, ConfirmDialog, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented } from "./ui";

type Dialog = { kind: "create" } | { kind: "import" } | { kind: "export"; key: SshKey } | { kind: "passphrase"; key: SshKey } | { kind: "delete"; key: SshKey };

export function KeysView() {
  const { ws, vault, update, toast } = useStore();
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const keys = ws.keys.filter((k) => k.vaultId === vault.id);

  const usage = (k: SshKey) => ws.hosts.filter((h) => h.auth.type === "key" && h.auth.keyId === k.id).length;

  const copyPublic = async (k: SshKey) =>
    toast((await copyText(k.publicKey)) ? { text: `Публичный ключ «${k.name}» скопирован`, tone: "success" } : { text: "Не удалось скопировать", tone: "error" });

  const savePublic = async (k: SshKey) => {
    const where = await api.saveTextFile(`${k.name}.pub`, `${k.publicKey}\n`);
    if (where) toast({ text: `Сохранено: ${where}`, tone: "success" });
  };

  const remove = (k: SshKey) => {
    update((w) => ({ ...w, keys: w.keys.filter((x) => x.id !== k.id) }));
    toast({ text: `Ключ «${k.name}» удалён`, action: { label: "Вернуть", run: () => update((w) => ({ ...w, keys: [...w.keys, k] })) } });
  };

  const create = () => setDialog({ kind: "create" });
  const close = () => setDialog(null);

  return (
    <div className="page">
      <PageHeader title="Ключи" count={keys.length || undefined}>
        <Button icon={Upload} onClick={() => setDialog({ kind: "import" })}>Импортировать</Button>
        <Button variant="primary" icon={Plus} onClick={create}>Создать ключ</Button>
      </PageHeader>

      {keys.length === 0 && <Empty icon={KeyRound} title="Нет ключей" action={<Button variant="primary" icon={Plus} onClick={create}>Создать ключ</Button>} />}

      <div className="rows">
        {keys.map((k) => {
          const n = usage(k);
          return (
            <div className="item-row" key={k.id}>
              <span className="item-icon"><KeyRound size={15} /></span>
              <div className="item-main">
                <span className="item-title">
                  {k.name}
                  <span className="chip">{k.algorithm === "ed25519" ? "Ed25519" : "RSA 4096"}</span>
                  {k.hasPassphrase && (
                    <span className="chip" title={k.passphrase ? "Пароль сохранён" : "Пароль спрашивается при подключении"}>
                      <Lock size={10} /> {k.passphrase ? "пароль сохранён" : "с паролем"}
                    </span>
                  )}
                </span>
                <span className="item-sub mono">{k.fingerprint}</span>
              </div>
              <span className="item-meta">{n ? `${n} ${plural(n, "хост", "хоста", "хостов")}` : "Не используется"}</span>
              <div className="row-actions visible">
                <IconButton icon={Copy} label="Копировать публичный ключ" tipSide="left" onClick={() => copyPublic(k)} />
                <Menu
                  trigger={(toggle, open) => <IconButton icon={Ellipsis} label="Ещё" className={open ? "pressed" : ""} onClick={toggle} />}
                  items={[
                    { label: "Сохранить публичный ключ", icon: Download, onSelect: () => savePublic(k) },
                    { label: "Экспортировать приватный ключ", icon: FileUp, onSelect: () => setDialog({ kind: "export", key: k }) },
                    { label: k.hasPassphrase ? "Изменить пароль ключа" : "Задать пароль ключа", icon: Lock, onSelect: () => setDialog({ kind: "passphrase", key: k }) },
                    { label: "Удалить", icon: Trash2, danger: true, onSelect: () => (n ? setDialog({ kind: "delete", key: k }) : remove(k)) },
                  ]}
                />
              </div>
            </div>
          );
        })}
      </div>

      {dialog?.kind === "create" && <CreateKeyDialog onClose={close} />}
      {dialog?.kind === "import" && <ImportKeyDialog onClose={close} />}
      {dialog?.kind === "export" && <ExportKeyDialog keyItem={dialog.key} onClose={close} />}
      {dialog?.kind === "passphrase" && <PassphraseDialog keyItem={dialog.key} onClose={close} />}
      {dialog?.kind === "delete" && (
        <ConfirmDialog
          title={`Удалить «${dialog.key.name}»?`}
          body={`Ключ используется в ${usage(dialog.key)} ${plural(usage(dialog.key), "хосте", "хостах", "хостах")}, они перестанут подключаться.`}
          confirm="Удалить"
          onConfirm={() => remove(dialog.key)}
          onClose={close}
        />
      )}
    </div>
  );
}

/* ---------- passphrase fields shared by create / import ---------- */

function PassphraseFields({ value, onChange, remember, onRemember, placeholder }: { value: string; onChange: (v: string) => void; remember: boolean; onRemember: (v: boolean) => void; placeholder: string }) {
  return (
    <>
      <Field label="Пароль ключа">
        <PasswordInput value={value} onChange={onChange} placeholder={placeholder} />
      </Field>
      {value && (
        <Checkbox checked={remember} onChange={onRemember}>
          Сохранить пароль в vault
        </Checkbox>
      )}
    </>
  );
}

/* ---------- create ---------- */

function CreateKeyDialog({ onClose }: { onClose: () => void }) {
  const { vault, update, toast } = useStore();
  const [name, setName] = useState(vault.kind === "team" ? `${vault.name.toLowerCase().replace(/\s+/g, "-")}-key` : "id_ed25519");
  const [algorithm, setAlgorithm] = useState<SshKey["algorithm"]>("ed25519");
  const [passphrase, setPassphrase] = useState("");
  const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    await sleep(algorithm === "ed25519" ? 400 : 1400);
    const key: SshKey = {
      id: uid("k"),
      vaultId: vault.id,
      name: name.trim(),
      algorithm,
      ...fakeKeyMaterial(algorithm, name.trim()),
      hasPassphrase: !!passphrase,
      passphrase: passphrase && remember ? passphrase : undefined,
      createdAt: "сегодня",
    };
    update((w) => ({ ...w, keys: [...w.keys, key] }));
    toast({ text: `Ключ «${key.name}» создан`, tone: "success", action: { label: "Копировать публичный", run: () => void copyText(key.publicKey) } });
    onClose();
  };

  return (
    <Modal
      title="Новый ключ"
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!name.trim()} loading={busy}>Создать</Button>
        </>
      }
    >
      <Field label="Название">
        <input className="input mono" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Тип">
        <Segmented value={algorithm} onChange={setAlgorithm} options={[{ value: "ed25519", label: "Ed25519" }, { value: "rsa-4096", label: "RSA 4096" }]} />
      </Field>
      <PassphraseFields value={passphrase} onChange={setPassphrase} remember={remember} onRemember={setRemember} placeholder="Без пароля" />
    </Modal>
  );
}

/* ---------- import ---------- */

function ImportKeyDialog({ onClose }: { onClose: () => void }) {
  const { vault, update, toast } = useStore();
  const [text, setText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [remember, setRemember] = useState(true);
  const [dragging, setDragging] = useState(false);
  const [showErrors, setShowErrors] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const parsed = parsePrivateKey(text);
  const autoName = text ? suggestKeyName(text, fileName ?? undefined) : "";

  const load = async (file: File) => {
    if (file.size > 64 * 1024) {
      toast({ text: "Файл слишком большой для ключа", tone: "error" });
      return;
    }
    setText(await file.text());
    setFileName(file.name);
    setShowErrors(true);
  };

  const submit = () => {
    setShowErrors(true);
    if (!parsed.ok) return;
    const keyName = name.trim() || autoName;
    const material = fakeKeyMaterial(parsed.algorithm, keyName);
    const key: SshKey = {
      id: uid("k"),
      vaultId: vault.id,
      name: keyName,
      algorithm: parsed.algorithm,
      fingerprint: material.fingerprint,
      publicKey: material.publicKey,
      privateKey: text.trim(),
      hasPassphrase: !!passphrase,
      passphrase: passphrase && remember ? passphrase : undefined,
      createdAt: "сегодня",
    };
    update((w) => ({ ...w, keys: [...w.keys, key] }));
    toast({ text: `Ключ «${key.name}» импортирован`, tone: "success" });
    onClose();
  };

  return (
    <Modal
      title="Импорт ключа"
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!text.trim()}>Импортировать</Button>
        </>
      }
    >
      <div
        className={`dropzone ${dragging ? "over" : ""} ${showErrors && !parsed.ok ? "invalid" : ""}`}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          const f = e.dataTransfer.files[0];
          if (f) void load(f);
        }}
      >
        <textarea
          className="input mono textarea key-text"
          rows={6}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setFileName(null);
          }}
          onBlur={() => text && setShowErrors(true)}
          placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
          spellCheck={false}
        />
        <div className="dropzone-bar">
          <Button type="button" size="sm" icon={FileUp} onClick={() => fileRef.current?.click()}>Выбрать файл</Button>
          <span className="muted small ellipsis">{fileName ?? "или перетащите файл сюда"}</span>
          <input
            ref={fileRef}
            type="file"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void load(f);
              e.target.value = "";
            }}
          />
        </div>
      </div>
      {showErrors && !parsed.ok && text && <span className="field-error">{parsed.error}</span>}
      {parsed.ok && <span className="ok-text small">{parsed.algorithm === "ed25519" ? "Ed25519" : "RSA"}, формат OpenSSH</span>}

      <Field label="Название">
        <input className="input mono" value={name} onChange={(e) => setName(e.target.value)} placeholder={autoName || "my-key"} />
      </Field>
      <PassphraseFields value={passphrase} onChange={setPassphrase} remember={remember} onRemember={setRemember} placeholder="Если ключ защищён паролем" />
    </Modal>
  );
}

/* ---------- export ---------- */

function ExportKeyDialog({ keyItem, onClose }: { keyItem: SshKey; onClose: () => void }) {
  const { toast } = useStore();
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    const where = await api.saveTextFile(keyItem.name, keyItem.privateKey);
    setBusy(false);
    if (!where) return;
    toast({ text: `Сохранено: ${where}`, tone: "success" });
    onClose();
  };

  return (
    <Modal
      title={`Экспорт «${keyItem.name}»`}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" icon={Download} loading={busy}>Сохранить файл</Button>
        </>
      }
    >
      {keyItem.hasPassphrase ? (
        <div className="modal-text">Файл будет защищён текущим паролем ключа.</div>
      ) : (
        <Field label="Пароль для файла">
          <PasswordInput value={passphrase} onChange={setPassphrase} placeholder="Без пароля" />
        </Field>
      )}
    </Modal>
  );
}

/* ---------- passphrase ---------- */

function PassphraseDialog({ keyItem, onClose }: { keyItem: SshKey; onClose: () => void }) {
  const { update, toast } = useStore();
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [remember, setRemember] = useState(!!keyItem.passphrase || !keyItem.hasPassphrase);
  const [showErrors, setShowErrors] = useState(false);

  const mismatch = next !== repeat;
  const removing = keyItem.hasPassphrase && !next;

  const submit = () => {
    setShowErrors(true);
    if (mismatch) return;
    update((w) => ({
      ...w,
      keys: w.keys.map((k) => (k.id === keyItem.id ? { ...k, hasPassphrase: !!next, passphrase: next && remember ? next : undefined } : k)),
    }));
    toast({ text: removing ? "Пароль ключа снят" : "Пароль ключа изменён", tone: "success" });
    onClose();
  };

  return (
    <Modal
      title={`Пароль ключа «${keyItem.name}»`}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant={removing ? "danger" : "primary"} disabled={!keyItem.hasPassphrase && !next}>
            {removing ? "Снять пароль" : "Сохранить"}
          </Button>
        </>
      }
    >
      <Field label="Новый пароль">
        <PasswordInput value={next} onChange={setNext} placeholder={keyItem.hasPassphrase ? "Пусто: снять пароль" : ""} />
      </Field>
      {next && (
        <>
          <Field label="Ещё раз" error={showErrors && mismatch ? "Пароли не совпадают" : null}>
            <PasswordInput value={repeat} onChange={setRepeat} invalid={showErrors && mismatch} />
          </Field>
          <Checkbox checked={remember} onChange={setRemember}>
            Сохранить пароль в vault
          </Checkbox>
        </>
      )}
    </Modal>
  );
}
