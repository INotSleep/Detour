import { useState, type ReactNode } from "react";
import type { Auth, Host, Proxy, SshKey } from "../lib/types";
import { Button, Checkbox, Field, PasswordInput, Segmented } from "./ui";
import { isNoGroup } from "./common";
import { useStore } from "../lib/store";
import { ProxyDialog, proxySummary } from "./ProxiesView";
import { tr, useT } from "../i18n";

/** The proxy written into a host by an older version, shown as a choice of its own. */
const LEGACY = "legacy";
/** The choice that makes a new proxy. */
const NEW = "+new";

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
  proxyType: "none" | "jump" | "proxy";
  jumpHostId: string;
  /** A saved proxy's id, or LEGACY for the one written into the host. */
  proxyId: string;
  /** The host's route as it was, kept while it is not changed (older versions read it). */
  wasProxy: Proxy | null;
  wasProxyId: string;
  forwardAgent: boolean;
}

function proxyFields(h: Pick<Host, "proxy" | "proxyId"> | null) {
  const p = h?.proxy ?? null;
  const legacy = p && p.type !== "jump" ? p : null;
  const proxyId = h?.proxyId ?? (legacy ? LEGACY : "");
  return {
    proxyType: proxyId ? ("proxy" as const) : p?.type === "jump" ? ("jump" as const) : ("none" as const),
    jumpHostId: p?.type === "jump" ? p.hostId : "",
    proxyId,
    wasProxy: p,
    wasProxyId: proxyId,
  };
}

export function draftFromHost(h: Host): HostDraft {
  return {
    label: h.label,
    address: h.address,
    port: String(h.port),
    user: h.user,
    group: isNoGroup(h.group) ? "" : h.group,
    authType: h.auth.type,
    keyId: h.auth.type === "key" ? h.auth.keyId : "",
    tags: h.tags.join(", "),
    password: "",
    forwardAgent: !!h.forwardAgent,
    ...proxyFields(h),
  };
}

export function emptyDraft(keys: SshKey[], group = ""): HostDraft {
  return { label: "", address: "", port: "22", user: "root", group, authType: keys.length ? "key" : "password", keyId: keys[0]?.id ?? "", tags: "", password: "", forwardAgent: false, ...proxyFields(null) };
}

export function draftErrors(d: HostDraft) {
  const port = Number(d.port);
  const t = tr().hosts.form.errors;
  return {
    address: d.address.trim() ? null : t.address,
    port: Number.isInteger(port) && port > 0 && port < 65536 ? null : t.portRange,
    jump: d.proxyType === "jump" && !d.jumpHostId ? t.chooseHost : null,
    proxy: d.proxyType === "proxy" && !d.proxyId ? t.chooseProxy : null,
  };
}

export const draftInvalid = (d: HostDraft) => Object.values(draftErrors(d)).some(Boolean);

/** The way to the host: a jump host, a saved proxy, or the proxy it had if that wasn't changed. */
function routeOf(d: HostDraft): Pick<Host, "proxy" | "proxyId"> {
  if (d.proxyType === "jump") return { proxy: { type: "jump", hostId: d.jumpHostId }, proxyId: null };
  if (d.proxyType !== "proxy") return { proxy: null, proxyId: null };
  if (d.proxyId === LEGACY) return { proxy: d.wasProxy, proxyId: null };
  // Unchanged: older versions on other devices keep reaching it the way they know.
  const kept = d.proxyId === d.wasProxyId && d.wasProxy?.type !== "jump" ? d.wasProxy : null;
  return { proxy: kept, proxyId: d.proxyId };
}

/** The typed host password is not part of the host; it goes to the vault via api.hostSetPassword. */
export function applyDraft(d: HostDraft): Pick<Host, "label" | "address" | "port" | "user" | "group" | "auth" | "tags" | "proxy" | "proxyId" | "forwardAgent"> {
  return {
    ...routeOf(d),
    label: d.label.trim() || d.address.trim(),
    address: d.address.trim(),
    port: Number(d.port),
    user: d.user.trim() || "root",
    group: d.group.trim(),
    auth: d.authType === "key" ? { type: "key", keyId: d.keyId } : { type: d.authType },
    tags: d.tags.split(",").map((t) => t.trim()).filter(Boolean),
    forwardAgent: d.forwardAgent,
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
}

export function HostFields({ draft, set, keys, groups, showErrors, passwordSaved, onForgetPassword, savedPassword, jumpHosts }: HostFieldsProps) {
  const t = useT();
  const { ws, vault } = useStore();
  const [newProxy, setNewProxy] = useState(false);
  const proxies = ws.proxies.filter((p) => p.vaultId === vault.id);
  const errors = draftErrors(draft);
  const err = (e: string | null) => (showErrors ? e : null);
  const legacy = draft.wasProxy && draft.wasProxy.type !== "jump" ? draft.wasProxy : null;
  return (
    <>
      <div className="row">
        <Field label={t.hosts.form.address} error={showErrors ? errors.address : null}>
          <input className={`input mono ${showErrors && errors.address ? "invalid" : ""}`} value={draft.address} onChange={(e) => set({ address: e.target.value })} placeholder="10.0.0.5" autoComplete="off" spellCheck={false} />
        </Field>
        <Field label={t.hosts.form.port} error={showErrors ? errors.port : null}>
          <input className={`input mono port ${showErrors && errors.port ? "invalid" : ""}`} value={draft.port} inputMode="numeric" onChange={(e) => set({ port: e.target.value.replace(/\D/g, "").slice(0, 5) })} />
        </Field>
      </div>
      <div className="row">
        <Field label={t.hosts.form.user}>
          <input className="input mono" value={draft.user} onChange={(e) => set({ user: e.target.value })} autoComplete="off" spellCheck={false} />
        </Field>
        <Field label={t.hosts.form.label}>
          <input className="input" value={draft.label} onChange={(e) => set({ label: e.target.value })} placeholder={draft.address || "web-1"} />
        </Field>
      </div>
      <Field label={t.hosts.form.auth}>
        <Segmented
          value={draft.authType}
          onChange={(v) => set({ authType: v, keyId: v === "key" && !draft.keyId ? keys[0]?.id ?? "" : draft.keyId })}
          options={[
            { value: "key", label: t.common.key },
            { value: "password", label: t.common.password },
            { value: "agent", label: "ssh-agent" },
          ]}
        />
      </Field>
      {draft.authType === "key" && (
        <Field label={t.common.key}>
          <select className="input" value={draft.keyId} onChange={(e) => set({ keyId: e.target.value })}>
            {keys.map((k) => (
              <option key={k.id} value={k.id}>{k.name}</option>
            ))}
          </select>
        </Field>
      )}
      {draft.authType === "password" && (
        <Field label={t.common.password}>
          <div className="input-group">
            <PasswordInput value={draft.password} onChange={(password) => set({ password })} placeholder={passwordSaved ? t.hosts.form.passwordSaved : t.hosts.form.askOnConnect} />
            {passwordSaved && !draft.password && onForgetPassword && (
              <Button type="button" variant="ghost" size="sm" onClick={onForgetPassword}>{t.hosts.form.forget}</Button>
            )}
          </div>
          {passwordSaved && !draft.password && savedPassword}
        </Field>
      )}
      <Field label={t.hosts.form.connection}>
        <Segmented
          value={draft.proxyType}
          onChange={(proxyType) => {
            // With no proxy to choose yet, "via proxy" makes one.
            if (proxyType === "proxy" && !proxies.length && !legacy) return setNewProxy(true);
            set({
              proxyType,
              jumpHostId: proxyType === "jump" && !draft.jumpHostId ? jumpHosts[0]?.id ?? "" : draft.jumpHostId,
              proxyId: proxyType === "proxy" && !draft.proxyId ? proxies[0]?.id ?? LEGACY : draft.proxyId,
            });
          }}
          options={[
            { value: "none", label: t.hosts.form.direct },
            ...(jumpHosts.length ? [{ value: "jump" as const, label: t.hosts.form.viaHost }] : []),
            { value: "proxy", label: t.hosts.form.viaProxy },
          ]}
        />
      </Field>
      {newProxy && <ProxyDialog proxy={null} onClose={() => setNewProxy(false)} onCreated={(proxyId) => set({ proxyType: "proxy", proxyId })} />}
      {draft.proxyType === "jump" && (
        <Field label={t.hosts.form.jumpHost} error={err(errors.jump)}>
          <select className="input" value={draft.jumpHostId} onChange={(e) => set({ jumpHostId: e.target.value })}>
            {jumpHosts.map((h) => (
              <option key={h.id} value={h.id}>{h.label} ({h.user}@{h.address})</option>
            ))}
          </select>
        </Field>
      )}
      {draft.proxyType === "proxy" && (
        <Field label={t.hosts.form.proxy} error={err(errors.proxy)}>
          <select className="input" value={draft.proxyId} onChange={(e) => (e.target.value === NEW ? setNewProxy(true) : set({ proxyId: e.target.value }))}>
            {!draft.proxyId && <option value="" />}
            {legacy && <option value={LEGACY}>{legacy.type.toUpperCase()} {proxySummary(legacy)}</option>}
            {proxies.map((p) => (
              <option key={p.id} value={p.id}>{p.name} ({t.proxies.types[p.kind.type]})</option>
            ))}
            <option value={NEW}>{t.hosts.form.newProxy}</option>
          </select>
        </Field>
      )}
      <div className="field">
        <Checkbox checked={draft.forwardAgent} onChange={(forwardAgent) => set({ forwardAgent })}>{t.hosts.form.forwardAgent}</Checkbox>
        {draft.forwardAgent && <span className="small warn-text">{t.hosts.form.forwardAgentRisk}</span>}
      </div>
      <div className="row">
        <Field label={t.hosts.form.group}>
          <input className="input" list="host-groups" value={draft.group} onChange={(e) => set({ group: e.target.value })} placeholder={t.hosts.form.noGroup} />
          <datalist id="host-groups">
            {groups.map((g) => <option key={g} value={g} />)}
          </datalist>
        </Field>
        <Field label={t.hosts.form.tags}>
          <input className="input" value={draft.tags} onChange={(e) => set({ tags: e.target.value })} placeholder="prod, api" />
        </Field>
      </div>
    </>
  );
}
