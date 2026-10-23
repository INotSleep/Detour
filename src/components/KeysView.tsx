import { useRef, useState } from "react";
import { Copy, Download, Ellipsis, FileUp, KeyRound, Lock, Plus, Trash2, Upload } from "lucide-react";
import { api, errorText, isCoreError } from "../lib/api";
import { parsePrivateKey, suggestKeyName } from "../lib/keys";
import { copyText, formatDate, useStore } from "../lib/store";
import type { SshKey, Workspace } from "../lib/types";
import { atLeast, writable } from "../lib/teams";
import { Button, Checkbox, ConfirmDialog, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented } from "./ui";
import { useT } from "../i18n";

type Dialog = { kind: "create" } | { kind: "import" } | { kind: "export"; key: SshKey } | { kind: "passphrase"; key: SshKey } | { kind: "delete"; key: SshKey };

export function algorithmLabel(k: Pick<SshKey, "algorithm" | "bits">) {
  if (k.algorithm === "ed25519") return "Ed25519";
  if (k.algorithm === "ecdsa") return "ECDSA";
  return k.bits ? `RSA ${k.bits}` : "RSA";
}

export function KeysView() {
  const t = useT();
  const { ws, vault, update, toast } = useStore();
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const canCreate = writable(vault);
  const keys = ws.keys.filter((k) => k.vaultId === vault.id);

  const usage = (k: SshKey) => ws.hosts.filter((h) => h.auth.type === "key" && h.auth.keyId === k.id).length;

  const copyPublic = async (k: SshKey) =>
    toast((await copyText(k.publicKey)) ? { text: t.keys.publicCopied(k.name), tone: "success" } : { text: t.common.copyFailed, tone: "error" });

  const savePublic = async (k: SshKey) => {
    try {
      const where = await api.saveTextFile(`${k.name}.pub`, `${k.publicKey}\n`);
      if (where) toast({ text: t.keys.saved(where), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const remove = (k: SshKey) => {
    update((w) => ({ ...w, keys: w.keys.filter((x) => x.id !== k.id) }));
    toast({ text: t.keys.removed(k.name), action: { label: t.keys.restore, run: () => update((w) => ({ ...w, keys: [...w.keys, k] })) } });
  };

  const create = () => setDialog({ kind: "create" });
  const close = () => setDialog(null);

  return (
    <div className="page">
      <PageHeader title={t.keys.title} count={keys.length || undefined}>
        {canCreate && <Button icon={Upload} onClick={() => setDialog({ kind: "import" })}>{t.keys.import}</Button>}
        {canCreate && <Button variant="primary" icon={Plus} onClick={create}>{t.keys.createKey}</Button>}
      </PageHeader>

      {keys.length === 0 && <Empty icon={KeyRound} title={t.keys.empty} action={canCreate ? <Button variant="primary" icon={Plus} onClick={create}>{t.keys.createKey}</Button> : undefined} />}

      <div className="rows">
        {keys.map((k) => {
          const n = usage(k);
          return (
            <div className="item-row" key={k.id}>
              <span className="item-icon"><KeyRound size={15} /></span>
              <div className="item-main">
                <span className="item-title">
                  {k.name}
                  <span className="chip">{algorithmLabel(k)}</span>
                  {k.hasPassphrase && (
                    <span className="chip">
                      <Lock size={10} /> {k.passphraseSaved ? t.keys.passphraseSaved : t.keys.withPassphrase}
                    </span>
                  )}
                </span>
                <span className="item-sub mono">{k.fingerprint}</span>
              </div>
              <span className="item-meta">{n ? t.common.hostCount(n) : formatDate(k.createdAt)}</span>
              <div className="row-actions visible">
                <IconButton icon={Copy} label={t.keys.copyPublic} tipSide="left" onClick={() => copyPublic(k)} />
                <Menu
                  trigger={(toggle, open) => <IconButton icon={Ellipsis} label={t.keys.more} className={open ? "pressed" : ""} onClick={toggle} />}
                  items={[
                    { label: t.keys.savePublic, icon: Download, onSelect: () => savePublic(k) },
                    ...(atLeast(k.access, "secret") ? [{ label: t.keys.exportPrivate, icon: FileUp, onSelect: () => setDialog({ kind: "export", key: k }) }] : []),
                    ...(atLeast(k.access, "edit")
                      ? [
                          { label: k.hasPassphrase ? t.keys.changePassphrase : t.keys.setPassphrase, icon: Lock, onSelect: () => setDialog({ kind: "passphrase", key: k }) },
                          { label: t.common.delete, icon: Trash2, danger: true, onSelect: () => (n ? setDialog({ kind: "delete", key: k }) : remove(k)) },
                        ]
                      : []),
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
          title={t.keys.deleteConfirm.title(dialog.key.name)}
          body={t.keys.deleteConfirm.body(usage(dialog.key))}
          confirm={t.common.delete}
          onConfirm={() => remove(dialog.key)}
          onClose={close}
        />
      )}
    </div>
  );
}

/* ---------- passphrase fields shared by create / import ---------- */

function PassphraseFields({ value, onChange, remember, onRemember, placeholder, error }: { value: string; onChange: (v: string) => void; remember: boolean; onRemember: (v: boolean) => void; placeholder: string; error?: string | null }) {
  const t = useT();
  return (
    <>
      <Field label={t.keys.passphrase} error={error}>
        <PasswordInput value={value} onChange={onChange} placeholder={placeholder} invalid={!!error} />
      </Field>
      {value && (
        <Checkbox checked={remember} onChange={onRemember}>
          {t.keys.savePassphrase}
        </Checkbox>
      )}
    </>
  );
}

const addKey = (k: SshKey) => (w: Workspace) => ({ ...w, keys: [...w.keys, k] });

/* ---------- create ---------- */

function CreateKeyDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { vault, update, toast } = useStore();
  const [name, setName] = useState(vault.kind === "team" ? `${vault.name.toLowerCase().replace(/\s+/g, "-")}-key` : "id_ed25519");
  const [algorithm, setAlgorithm] = useState<"ed25519" | "rsa">("ed25519");
  const [passphrase, setPassphrase] = useState("");
  const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    try {
      const key = await api.keyGenerate(vault.id, name.trim(), algorithm, passphrase || null, remember);
      update(addKey(key));
      toast({ text: t.keys.create.created(key.name), tone: "success", action: { label: t.keys.create.copyPublic, run: () => void copyText(key.publicKey) } });
      onClose();
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t.keys.create.title}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!name.trim()} loading={busy}>{t.common.create}</Button>
        </>
      }
    >
      <Field label={t.keys.name}>
        <input className="input mono" value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label={t.keys.create.type}>
        <Segmented value={algorithm} onChange={setAlgorithm} options={[{ value: "ed25519", label: "Ed25519" }, { value: "rsa", label: "RSA 4096" }]} />
      </Field>
      <PassphraseFields value={passphrase} onChange={setPassphrase} remember={remember} onRemember={setRemember} placeholder={t.keys.noPassphrase} />
    </Modal>
  );
}

/* ---------- import ---------- */

function ImportKeyDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { vault, update, toast } = useStore();
  const [text, setText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [remember, setRemember] = useState(true);
  const [dragging, setDragging] = useState(false);
  const [showErrors, setShowErrors] = useState(false);
  const [busy, setBusy] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [passError, setPassError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const parsed = parsePrivateKey(text);
  const autoName = text ? suggestKeyName(text, fileName ?? undefined) : "";

  const setKeyText = (value: string, file: string | null) => {
    setText(value);
    setFileName(file);
    setKeyError(null);
    setPassError(null);
  };

  const load = async (file: File) => {
    if (file.size > 64 * 1024) {
      toast({ text: t.keys.importDialog.fileTooBig, tone: "error" });
      return;
    }
    setKeyText(await file.text(), file.name);
    setShowErrors(true);
  };

  const submit = async () => {
    setShowErrors(true);
    if (!parsed.ok || busy) return;
    setBusy(true);
    try {
      const key = await api.keyImport(vault.id, name.trim() || autoName, text, passphrase || null, remember);
      update(addKey(key));
      toast({ text: t.keys.importDialog.imported(key.name), tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      if (isCoreError(e) && e.kind === "passphraseRequired") setPassError(t.keys.importDialog.passphraseRequired);
      else if (isCoreError(e) && e.kind === "wrongPassphrase") setPassError(t.common.errors.wrongPassphrase);
      else setKeyError(errorText(e));
    }
  };

  const shownKeyError = keyError ?? (showErrors && !parsed.ok && text ? parsed.error : null);

  return (
    <Modal
      title={t.keys.importDialog.title}
      width={520}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!text.trim()} loading={busy}>{t.keys.import}</Button>
        </>
      }
    >
      <div
        className={`dropzone ${dragging ? "over" : ""} ${shownKeyError ? "invalid" : ""}`}
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
          onChange={(e) => setKeyText(e.target.value, null)}
          onBlur={() => text && setShowErrors(true)}
          placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
          spellCheck={false}
        />
        <div className="dropzone-bar">
          <Button type="button" size="sm" icon={FileUp} onClick={() => fileRef.current?.click()}>{t.keys.importDialog.chooseFile}</Button>
          <span className="muted small ellipsis">{fileName ?? t.keys.importDialog.orDrop}</span>
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
      {shownKeyError && <span className="field-error">{shownKeyError}</span>}
      {!shownKeyError && parsed.ok && <span className="ok-text small">{t.keys.importDialog.detected(parsed.algorithm === "rsa" ? "RSA" : "OpenSSH")}</span>}

      <Field label={t.keys.name}>
        <input className="input mono" value={name} onChange={(e) => setName(e.target.value)} placeholder={autoName || "my-key"} />
      </Field>
      <PassphraseFields
        value={passphrase}
        onChange={(v) => {
          setPassphrase(v);
          setPassError(null);
        }}
        remember={remember}
        onRemember={setRemember}
        placeholder={t.keys.importDialog.passphrasePlaceholder}
        error={passError}
      />
    </Modal>
  );
}

/* ---------- export ---------- */

function ExportKeyDialog({ keyItem, onClose }: { keyItem: SshKey; onClose: () => void }) {
  const t = useT();
  const { toast } = useStore();
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const contents = await api.keyExport(keyItem, passphrase || null);
      const where = await api.saveTextFile(keyItem.name, contents);
      setBusy(false);
      if (!where) return;
      toast({ text: t.keys.saved(where), tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      toast({ text: errorText(e), tone: "error" });
    }
  };

  return (
    <Modal
      title={t.keys.export.title(keyItem.name)}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" icon={Download} loading={busy}>{t.keys.export.saveFile}</Button>
        </>
      }
    >
      {keyItem.hasPassphrase ? (
        <div className="modal-text">{t.keys.export.keepsPassphrase}</div>
      ) : (
        <Field label={t.keys.export.filePassphrase}>
          <PasswordInput value={passphrase} onChange={setPassphrase} placeholder={t.keys.noPassphrase} />
        </Field>
      )}
    </Modal>
  );
}

/* ---------- passphrase ---------- */

function PassphraseDialog({ keyItem, onClose }: { keyItem: SshKey; onClose: () => void }) {
  const t = useT();
  const { update, toast } = useStore();
  const needsCurrent = keyItem.hasPassphrase && !keyItem.passphraseSaved;
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [remember, setRemember] = useState(keyItem.passphraseSaved || !keyItem.hasPassphrase);
  const [showErrors, setShowErrors] = useState(false);
  const [currentError, setCurrentError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const mismatch = next !== repeat;
  const removing = keyItem.hasPassphrase && !next;

  const submit = async () => {
    setShowErrors(true);
    if (mismatch || busy || (needsCurrent && !current)) return;
    setBusy(true);
    try {
      const updated = await api.keySetPassphrase(keyItem, current || null, next || null, remember);
      update((w) => ({ ...w, keys: w.keys.map((k) => (k.id === keyItem.id ? updated : k)) }));
      toast({ text: removing ? t.keys.passphraseDialog.removed : t.keys.passphraseDialog.changed, tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      if (isCoreError(e) && (e.kind === "wrongPassphrase" || e.kind === "passphraseRequired")) setCurrentError(t.keys.passphraseDialog.wrongCurrent);
      else toast({ text: errorText(e), tone: "error" });
    }
  };

  return (
    <Modal
      title={t.keys.passphraseDialog.title(keyItem.name)}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant={removing ? "danger" : "primary"} loading={busy} disabled={(!keyItem.hasPassphrase && !next) || (needsCurrent && !current)}>
            {removing ? t.keys.passphraseDialog.remove : t.common.save}
          </Button>
        </>
      }
    >
      {needsCurrent && (
        <Field label={t.keys.passphraseDialog.current} error={currentError}>
          <PasswordInput value={current} onChange={(v) => { setCurrent(v); setCurrentError(null); }} invalid={!!currentError} autoFocus />
        </Field>
      )}
      <Field label={t.keys.passphraseDialog.next}>
        <PasswordInput value={next} onChange={setNext} placeholder={keyItem.hasPassphrase ? t.keys.passphraseDialog.emptyRemoves : ""} />
      </Field>
      {next && (
        <>
          <Field label={t.keys.passphraseDialog.repeat} error={showErrors && mismatch ? t.keys.passphraseDialog.mismatch : null}>
            <PasswordInput value={repeat} onChange={setRepeat} invalid={showErrors && mismatch} />
          </Field>
          <Checkbox checked={remember} onChange={setRemember}>
            {t.keys.savePassphrase}
          </Checkbox>
        </>
      )}
    </Modal>
  );
}
