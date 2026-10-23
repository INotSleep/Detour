import { useState, type ReactNode } from "react";
import type { Auth, Host, Proxy, SshKey } from "../lib/types";
import { Button, Field, PasswordInput, Segmented } from "./ui";

export interface HostDraft {
  label: string;
  address: string;
  port: string;
  user: string;
  group: string;
  authType: Auth["type"];
  keyId: string;
  tags: string;
  password: string;
  proxyType: "none" | Proxy["type"];
  jumpHostId: string;
  proxyAddress: string;
  proxyPort: string;
  proxyUser: string;
  proxyPassword: string;
}

const proxyFields = (p: Proxy | null | undefined) => ({
  proxyType: p?.type ?? ("none" as const),
  jumpHostId: p?.type === "jump" ? p.hostId : "",
  proxyAddress: p && p.type !== "jump" ? p.address : "",
  proxyPort: p && p.type !== "jump" ? String(p.port) : "",
  proxyUser: p && p.type !== "jump" ? p.user : "",
  proxyPassword: "",
});

export function draftFromHost(h: Host): HostDraft {
  return {
    label: h.label,
    address: h.address,
    port: String(h.port),
    user: h.user,
    group: h.group,
    authType: h.auth.type,
    keyId: h.auth.type === "key" ? h.auth.keyId : "",
    tags: h.tags.join(", "),
    password: "",
    ...proxyFields(h.proxy),
  };
}

export function emptyDraft(keys: SshKey[], group = ""): HostDraft {
  return { label: "", address: "", port: "22", user: "root", group, authType: keys.length ? "key" : "password", keyId: keys[0]?.id ?? "", tags: "", password: "", ...proxyFields(null) };
}

export function draftErrors(d: HostDraft) {
  const port = Number(d.port);
  const proxyPort = Number(d.proxyPort);
  const viaProxy = d.proxyType === "socks5" || d.proxyType === "http";
  return {
    address: d.address.trim() ? null : "Укажите адрес",
    port: Number.isInteger(port) && port > 0 && port < 65536 ? null : "От 1 до 65535",
    jump: d.proxyType === "jump" && !d.jumpHostId ? "Выберите хост" : null,
    proxyAddress: viaProxy && !d.proxyAddress.trim() ? "Укажите адрес" : null,
    proxyPort: viaProxy && !(Number.isInteger(proxyPort) && proxyPort > 0 && proxyPort < 65536) ? "От 1 до 65535" : null,
  };
}

export const draftInvalid = (d: HostDraft) => Object.values(draftErrors(d)).some(Boolean);

function proxyOf(d: HostDraft): Proxy | null {
  if (d.proxyType === "jump") return { type: "jump", hostId: d.jumpHostId };
  if (d.proxyType === "socks5" || d.proxyType === "http") return { type: d.proxyType, address: d.proxyAddress.trim(), port: Number(d.proxyPort), user: d.proxyUser.trim() };
  return null;
}

/** The typed host password is not part of the host; it goes to the vault via api.hostSetPassword. A typed proxy password goes with the host. */
export function applyDraft(d: HostDraft): Pick<Host, "label" | "address" | "port" | "user" | "group" | "auth" | "tags" | "proxy" | "newProxyPassword" | "hasProxyPassword"> {
  const proxy = proxyOf(d);
  const typed = proxy && proxy.type !== "jump" && proxy.user && d.proxyPassword ? d.proxyPassword : undefined;
  return {
    proxy,
    ...(typed ? { newProxyPassword: typed, hasProxyPassword: true } : {}),
    label: d.label.trim() || d.address.trim(),
    address: d.address.trim(),
    port: Number(d.port),
    user: d.user.trim() || "root",
    group: d.group.trim() || "Без группы",
    auth: d.authType === "key" ? { type: "key", keyId: d.keyId } : { type: d.authType },
    tags: d.tags.split(",").map((t) => t.trim()).filter(Boolean),
  };
}

export function useHostDraft(initial: HostDraft) {
  const [draft, setDraft] = useState(initial);
  const [touched, setTouched] = useState(false);
  const set = (patch: Partial<HostDraft>) => setDraft((d) => ({ ...d, ...patch }));
  return { draft, set, setDraft, touched, setTouched };
}

interface HostFieldsProps {
  draft: HostDraft;
  set: (p: Partial<HostDraft>) => void;
  keys: SshKey[];
  groups: string[];
  showErrors: boolean;
  /** A password is already saved for this host. */
  passwordSaved?: boolean;
  onForgetPassword?: () => void;
  /** Shows the saved password on request. */
  savedPassword?: ReactNode;
  /** Hosts of the same vault this one may go through. */
  jumpHosts: Host[];
  /** A proxy password is already saved for this host. */
  proxyPasswordSaved?: boolean;
}

export function HostFields({ draft, set, keys, groups, showErrors, passwordSaved, onForgetPassword, savedPassword, jumpHosts, proxyPasswordSaved }: HostFieldsProps) {
  const errors = draftErrors(draft);
  const err = (e: string | null) => (showErrors ? e : null);
  return (
    <>
      <div className="row">
        <Field label="Адрес" error={showErrors ? errors.address : null}>
          <input className={`input mono ${showErrors && errors.address ? "invalid" : ""}`} value={draft.address} onChange={(e) => set({ address: e.target.value })} placeholder="10.0.0.5" autoComplete="off" spellCheck={false} />
        </Field>
        <Field label="Порт" error={showErrors ? errors.port : null}>
          <input className={`input mono port ${showErrors && errors.port ? "invalid" : ""}`} value={draft.port} inputMode="numeric" onChange={(e) => set({ port: e.target.value.replace(/\D/g, "").slice(0, 5) })} />
        </Field>
      </div>
      <div className="row">
        <Field label="Пользователь">
          <input className="input mono" value={draft.user} onChange={(e) => set({ user: e.target.value })} autoComplete="off" spellCheck={false} />
        </Field>
        <Field label="Название">
          <input className="input" value={draft.label} onChange={(e) => set({ label: e.target.value })} placeholder={draft.address || "web-1"} />
        </Field>
      </div>
      <Field label="Вход">
        <Segmented
          value={draft.authType}
          onChange={(v) => set({ authType: v, keyId: v === "key" && !draft.keyId ? keys[0]?.id ?? "" : draft.keyId })}
          options={[
            { value: "key", label: "Ключ" },
            { value: "password", label: "Пароль" },
            { value: "agent", label: "ssh-agent" },
          ]}
        />
      </Field>
      {draft.authType === "key" && (
        <Field label="Ключ">
          <select className="input" value={draft.keyId} onChange={(e) => set({ keyId: e.target.value })}>
            {keys.map((k) => (
              <option key={k.id} value={k.id}>{k.name}</option>
            ))}
          </select>
        </Field>
      )}
      {draft.authType === "password" && (
        <Field label="Пароль">
          <div className="input-group">
            <PasswordInput value={draft.password} onChange={(password) => set({ password })} placeholder={passwordSaved ? "Сохранён" : "Спрашивать при подключении"} />
            {passwordSaved && !draft.password && onForgetPassword && (
              <Button type="button" variant="ghost" size="sm" onClick={onForgetPassword}>Забыть</Button>
            )}
          </div>
          {passwordSaved && !draft.password && savedPassword}
        </Field>
      )}
      <Field label="Подключение">
        <Segmented
          value={draft.proxyType}
          onChange={(proxyType) =>
            set({
              proxyType,
              jumpHostId: proxyType === "jump" && !draft.jumpHostId ? jumpHosts[0]?.id ?? "" : draft.jumpHostId,
              // The usual ports of each kind of proxy, until one is typed.
              proxyPort: proxyType === "socks5" && (!draft.proxyPort || draft.proxyPort === "3128") ? "1080" : proxyType === "http" && (!draft.proxyPort || draft.proxyPort === "1080") ? "3128" : draft.proxyPort,
            })
          }
          options={[
            { value: "none", label: "Напрямую" },
            ...(jumpHosts.length ? [{ value: "jump" as const, label: "Через хост" }] : []),
            { value: "socks5", label: "SOCKS5" },
            { value: "http", label: "HTTP" },
          ]}
        />
      </Field>
      {draft.proxyType === "jump" && (
        <Field label="Промежуточный хост" error={err(errors.jump)}>
          <select className="input" value={draft.jumpHostId} onChange={(e) => set({ jumpHostId: e.target.value })}>
            {jumpHosts.map((h) => (
              <option key={h.id} value={h.id}>{h.label} ({h.user}@{h.address})</option>
            ))}
          </select>
        </Field>
      )}
      {(draft.proxyType === "socks5" || draft.proxyType === "http") && (
        <>
          <div className="row">
            <Field label="Адрес прокси" error={err(errors.proxyAddress)}>
              <input className={`input mono ${err(errors.proxyAddress) ? "invalid" : ""}`} value={draft.proxyAddress} onChange={(e) => set({ proxyAddress: e.target.value })} placeholder="proxy.corp.local" autoComplete="off" spellCheck={false} />
            </Field>
            <Field label="Порт" error={err(errors.proxyPort)}>
              <input className={`input mono port ${err(errors.proxyPort) ? "invalid" : ""}`} value={draft.proxyPort} inputMode="numeric" onChange={(e) => set({ proxyPort: e.target.value.replace(/\D/g, "").slice(0, 5) })} />
            </Field>
          </div>
          <div className="row">
            <Field label="Логин прокси">
              <input className="input mono" value={draft.proxyUser} onChange={(e) => set({ proxyUser: e.target.value })} placeholder="Без входа" autoComplete="off" spellCheck={false} />
            </Field>
            <Field label="Пароль прокси">
              <PasswordInput value={draft.proxyPassword} onChange={(proxyPassword) => set({ proxyPassword })} placeholder={proxyPasswordSaved ? "Сохранён" : ""} />
            </Field>
          </div>
        </>
      )}
      <div className="row">
        <Field label="Группа">
          <input className="input" list="host-groups" value={draft.group} onChange={(e) => set({ group: e.target.value })} placeholder="Без группы" />
          <datalist id="host-groups">
            {groups.map((g) => <option key={g} value={g} />)}
          </datalist>
        </Field>
        <Field label="Теги">
          <input className="input" value={draft.tags} onChange={(e) => set({ tags: e.target.value })} placeholder="prod, api" />
        </Field>
      </div>
    </>
  );
}
