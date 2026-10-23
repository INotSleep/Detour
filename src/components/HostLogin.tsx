// Pieces of the host panel about logging in: a saved password you may see,
// and your own login for a team host.

import { useState } from "react";
import { Copy, Eye, EyeOff } from "lucide-react";
import { api, errorText } from "../lib/api";
import { copyText, useStore } from "../lib/store";
import { atLeast } from "../lib/teams";
import { tr, useT } from "../i18n";
import type { Auth, Host } from "../lib/types";
import { Button, Field, IconButton, PasswordInput, Segmented } from "./ui";

/** A saved password, fetched from the vault only when asked for. */
export function SavedPassword({ host, own = false }: { host: Host; own?: boolean }) {
  const t = useT();
  const { toast } = useStore();
  const [value, setValue] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    if (value !== null) return value;
    setBusy(true);
    try {
      const pw = await api.hostPassword(host, own);
      setBusy(false);
      return pw;
    } catch (e) {
      setBusy(false);
      toast({ text: errorText(e), tone: "error" });
      return null;
    }
  };

  const toggle = async () => {
    if (value !== null) return setValue(null);
    setValue(await load());
  };

  const copy = async () => {
    const pw = await load();
    if (pw !== null) toast((await copyText(pw)) ? { text: tr().terminal.login.passwordCopied, tone: "success" } : { text: tr().common.copyFailed, tone: "error" });
  };

  return (
    <span className="saved-password">
      <span className={`mono ${value === null ? "muted" : ""}`}>{value ?? "••••••••"}</span>
      <IconButton type="button" icon={value === null ? Eye : EyeOff} label={value === null ? t.common.show : t.common.hide} size={14} disabled={busy} onClick={toggle} />
      <IconButton type="button" icon={Copy} label={t.common.copy} size={14} disabled={busy} onClick={copy} />
    </span>
  );
}

/** Your own login for a team host: used instead of the shared one, never leaves your personal vault. */
export function OwnLoginSection({ host }: { host: Host }) {
  const t = useT();
  const l = t.terminal.login;
  const { ws, vault, update, toast, applyTeamView } = useStore();
  const personalKeys = ws.keys.filter((k) => k.vaultId === "personal");
  const own = host.own;
  const shared = atLeast(host.access, "secret");
  const [editing, setEditing] = useState(false);
  const [user, setUser] = useState(own?.user ?? host.user);
  const [authType, setAuthType] = useState<Auth["type"]>(own?.auth.type ?? (personalKeys.length ? "key" : "password"));
  const [keyId, setKeyId] = useState(own?.auth.type === "key" ? own.auth.keyId : personalKeys[0]?.id ?? "");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  const keyName = own?.auth.type === "key" ? personalKeys.find((k) => k.id === (own.auth as { keyId: string }).keyId)?.name ?? t.common.key : null;
  const authText = own ? (own.auth.type === "key" ? keyName : own.auth.type === "agent" ? "ssh-agent" : own.hasPassword ? null : l.askOnConnect) : null;
  const keyMissing = authType === "key" && !keyId;

  const set = async (login: { user: string; auth: Auth; newPassword?: string } | null) => {
    setBusy(true);
    try {
      if (api.real) {
        applyTeamView(await api.teamOwnLoginSet(vault.id, host.id, login));
      } else {
        const next = login && { user: login.user, auth: login.auth, hasPassword: login.auth.type === "password" && (!!login.newPassword || !!own?.hasPassword) };
        update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, own: next ?? undefined } : h)) }));
      }
      toast({ text: login ? tr().terminal.login.saved : tr().terminal.login.removed, tone: "success" });
      setEditing(false);
      setPassword("");
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(false);
  };

  const save = () => {
    if (keyMissing || busy) return;
    const auth: Auth = authType === "key" ? { type: "key", keyId } : { type: authType };
    void set({ user: user.trim() || host.user, auth, newPassword: authType === "password" && password ? password : undefined });
  };

  if (!editing) {
    return (
      <section className="own-login">
        <h3 className="section-title">{l.title}</h3>
        {own ? (
          <div className="own-summary">
            <div className="own-line">
              <span className="mono ellipsis">{own.user}</span>
              {authText && <span className="muted ellipsis">{authText}</span>}
              {own.auth.type === "password" && own.hasPassword && <SavedPassword host={host} own />}
            </div>
            <div className="own-actions">
              <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>{t.common.edit}</Button>
              <Button size="sm" variant="ghost" loading={busy} onClick={() => set(null)}>{shared ? l.useShared : t.common.remove}</Button>
            </div>
          </div>
        ) : (
          <Button size="sm" onClick={() => setEditing(true)}>{shared ? l.useOwn : l.setOwn}</Button>
        )}
      </section>
    );
  }

  return (
    <form
      className="own-login panel-form"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
      onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), setEditing(false))}
    >
      <h3 className="section-title">{l.title}</h3>
      <Field label={l.user}>
        <input className="input mono" value={user} onChange={(e) => setUser(e.target.value)} autoComplete="off" spellCheck={false} autoFocus />
      </Field>
      <Field label={l.auth}>
        <Segmented
          value={authType}
          onChange={setAuthType}
          options={[
            { value: "key", label: l.ownKey },
            { value: "password", label: t.common.password },
            { value: "agent", label: "ssh-agent" },
          ]}
        />
      </Field>
      {authType === "key" && (
        <Field label={t.common.key} error={keyMissing ? l.noKeys : null}>
          <select className="input" value={keyId} onChange={(e) => setKeyId(e.target.value)} disabled={!personalKeys.length}>
            {personalKeys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
          </select>
        </Field>
      )}
      {authType === "password" && (
        <Field label={t.common.password}>
          <PasswordInput value={password} onChange={setPassword} placeholder={own?.hasPassword ? l.passwordSaved : l.askPassword} />
        </Field>
      )}
      <div className="panel-save">
        <Button type="button" variant="ghost" onClick={() => setEditing(false)}>{t.common.cancel}</Button>
        <Button type="submit" variant="primary" loading={busy} disabled={keyMissing}>{t.common.save}</Button>
      </div>
    </form>
  );
}
