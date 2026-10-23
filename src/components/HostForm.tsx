import { useState } from "react";
import type { Auth, Host, SshKey } from "../lib/types";
import { Field, PasswordInput, Segmented } from "./ui";

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
}

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
    password: h.password ?? "",
  };
}

export function emptyDraft(keys: SshKey[], group = ""): HostDraft {
  return { label: "", address: "", port: "22", user: "root", group, authType: keys.length ? "key" : "password", keyId: keys[0]?.id ?? "", tags: "", password: "" };
}

export function draftErrors(d: HostDraft) {
  const port = Number(d.port);
  return {
    address: d.address.trim() ? null : "Укажите адрес",
    port: Number.isInteger(port) && port > 0 && port < 65536 ? null : "От 1 до 65535",
  };
}

export function applyDraft(d: HostDraft): Pick<Host, "label" | "address" | "port" | "user" | "group" | "auth" | "tags" | "password"> {
  return {
    label: d.label.trim() || d.address.trim(),
    address: d.address.trim(),
    port: Number(d.port),
    user: d.user.trim() || "root",
    group: d.group.trim() || "Без группы",
    auth: d.authType === "key" ? { type: "key", keyId: d.keyId } : { type: d.authType },
    tags: d.tags.split(",").map((t) => t.trim()).filter(Boolean),
    password: d.authType === "password" && d.password ? d.password : undefined,
  };
}

export function useHostDraft(initial: HostDraft) {
  const [draft, setDraft] = useState(initial);
  const [touched, setTouched] = useState(false);
  const set = (patch: Partial<HostDraft>) => setDraft((d) => ({ ...d, ...patch }));
  return { draft, set, setDraft, touched, setTouched };
}

export function HostFields({ draft, set, keys, groups, showErrors }: { draft: HostDraft; set: (p: Partial<HostDraft>) => void; keys: SshKey[]; groups: string[]; showErrors: boolean }) {
  const errors = draftErrors(draft);
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
          <PasswordInput value={draft.password} onChange={(password) => set({ password })} placeholder="Спрашивать при подключении" />
        </Field>
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
