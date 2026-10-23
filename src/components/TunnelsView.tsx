import { useRef, useState } from "react";
import { ArrowLeftRight, ArrowRight, Copy, Ellipsis, Globe, Laptop, Loader2, Pencil, Plus, RotateCw, Server, ShieldQuestion, Trash2, type LucideIcon } from "lucide-react";
import { api, errorText, isCoreError, trustMore, type SecretKind } from "../lib/api";
import { copyText, sleep, uid, useStore } from "../lib/store";
import type { Host, Tunnel } from "../lib/types";
import { atLeast, writable } from "../lib/teams";
import { Button, Checkbox, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Switch } from "./ui";
import { tr, useT } from "../i18n";

/* ---------- route ---------- */

interface RouteNode {
  icon: LucideIcon;
  title: string;
  sub?: string;
}

function routeOf(kind: Tunnel["kind"], bindPort: number | string, via: string, target?: string): RouteNode[] {
  const port = bindPort || "…";
  const tx = tr().tunnels.route;
  const me = tx.thisComputer;
  if (kind === "local") {
    // A service on the server itself is shown as the server's own port, not as a separate "localhost" hop.
    const onServer = target?.match(/^(?:localhost|127\.0\.0\.1):(\d+)$/);
    const from = { icon: Laptop, title: me, sub: `localhost:${port}` };
    if (onServer) return [from, { icon: Server, title: via || "…", sub: tx.port(onServer[1]!) }];
    return [from, { icon: Server, title: via || "…" }, { icon: Server, title: target || "…" }];
  }
  if (kind === "remote") return [{ icon: Server, title: via || "…", sub: tx.port(port) }, { icon: Laptop, title: me, sub: target || "…" }];
  return [{ icon: Laptop, title: me, sub: `SOCKS5 localhost:${port}` }, { icon: Server, title: via || "…" }, { icon: Globe, title: tx.anyAddress }];
}

function Route({ nodes }: { nodes: RouteNode[] }) {
  return (
    <div className="route">
      {nodes.map((n, i) => (
        <span className="route-part" key={i}>
          {i > 0 && <ArrowRight size={12} className="route-arrow" />}
          <span className="route-node">
            <n.icon size={13} />
            <span className="route-text">
              <span className="route-title">{n.title}</span>
              {n.sub && <span className="route-sub">{n.sub}</span>}
            </span>
          </span>
        </span>
      ))}
    </div>
  );
}

/** What to copy to use an open tunnel. */
function endpoint(t: Tunnel) {
  if (t.kind === "local") return `localhost:${t.bindPort}`;
  if (t.kind === "dynamic") return `socks5://localhost:${t.bindPort}`;
  // The server listens on its own loopback: this is the address on the server.
  return `localhost:${t.bindPort}`;
}

/** What a tunnel's login asks the user for. */
type Ask =
  | { kind: "secret"; tunnel: Tunnel; secret: SecretKind; text: string; trust?: string; error?: string }
  | { kind: "trust"; tunnel: Tunnel; fingerprint: string; host: string; trusted?: string }
  | { kind: "changed"; tunnel: Tunnel; expected: string; actual: string; host: string; keyHostId: string; trusted?: string };

interface Answer {
  secret?: string;
  secretKind?: SecretKind;
  remember?: boolean;
  trust?: string;
}

/* ---------- page ---------- */

export function TunnelsView() {
  // `t` names a tunnel throughout this file.
  const tx = useT();
  const { ws, vault, update, toast, navigate } = useStore();
  const canCreate = writable(vault);
  const [editing, setEditing] = useState<Tunnel | "new" | null>(null);
  const [ask, setAsk] = useState<Ask | null>(null);
  // Each start attempt gets a token; a click while starting invalidates it, so a late result can't flip the switch.
  const attempts = useRef(new Map<string, number>());
  // Async start() and rapid clicks must see the latest state, not the render they were created in.
  const latest = useRef(ws);
  latest.current = ws;

  const tunnels = ws.tunnels.filter((t) => t.vaultId === vault.id);
  const hosts = ws.hosts.filter((h) => h.vaultId === vault.id);

  const patch = (id: string, p: Partial<Tunnel>) => update((w) => ({ ...w, tunnels: w.tunnels.map((t) => (t.id === id ? { ...t, ...p } : t)) }));

  const stop = (t: Tunnel) => {
    attempts.current.set(t.id, (attempts.current.get(t.id) ?? 0) + 1);
    patch(t.id, { state: "off", error: undefined, connections: 0 });
    if (api.real) void api.tunnelStop(t).catch(() => {});
  };

  const start = async (t: Tunnel, answer: Answer = {}) => {
    const token = (attempts.current.get(t.id) ?? 0) + 1;
    attempts.current.set(t.id, token);
    patch(t.id, { state: "starting", error: undefined, connections: 0 });
    if (api.real) return startReal(t, token, answer);
    await sleep(900);
    if (attempts.current.get(t.id) !== token) return;

    const now = latest.current;
    const via = now.hosts.find((h) => h.id === t.viaHostId);
    const busy = t.kind !== "remote" && now.tunnels.find((o) => o.id !== t.id && o.kind !== "remote" && o.state === "on" && o.bindPort === t.bindPort);
    if (!via || via.online === false) return patch(t.id, { state: "error", error: tx.common.errors.unreachable(via?.label ?? tx.tunnels.server) });
    if (busy) return patch(t.id, { state: "error", error: tx.tunnels.portTaken(t.bindPort, busy.name) });
    patch(t.id, { state: "on" });
  };

  const startReal = async (t: Tunnel, token: number, answer: Answer) => {
    const current = () => attempts.current.get(t.id) === token;
    try {
      await api.tunnelStart(t, answer.secret ?? null, answer.trust ?? null);
      // Switched off while it was opening.
      if (!current()) return void api.tunnelStop(t).catch(() => {});
      patch(t.id, { state: "on" });
      const host = latest.current.hosts.find((h) => h.id === t.viaHostId);
      if (answer.secret && answer.remember && answer.secretKind && host) {
        await api.secretSave(host, answer.secretKind, answer.secret).catch((e) => toast({ text: errorText(e), tone: "error" }));
      }
    } catch (e) {
      if (!current()) return;
      const via = latest.current.hosts.find((h) => h.id === t.viaHostId);
      if (isCoreError(e) && e.kind === "needSecret") {
        patch(t.id, { state: "off" });
        return setAsk({ kind: "secret", tunnel: t, secret: e.secret, text: secretLabel(e.secret, via), trust: answer.trust });
      }
      if (isCoreError(e) && e.kind === "authFailed" && answer.secret) {
        patch(t.id, { state: "off" });
        return setAsk({ kind: "secret", tunnel: t, secret: e.secret ?? "password", text: secretLabel(e.secret ?? "password", via), trust: answer.trust, error: errorText(e) });
      }
      if (isCoreError(e) && e.kind === "hostKeyUnknown") {
        patch(t.id, { state: "off" });
        const keyHost = e.via ? latest.current.hosts.find((h) => h.id === e.via) : via;
        return setAsk({ kind: "trust", tunnel: t, fingerprint: e.fingerprint, host: keyHost ? `${keyHost.address}:${keyHost.port}` : "", trusted: answer.trust });
      }
      if (isCoreError(e) && e.kind === "hostKeyChanged") {
        patch(t.id, { state: "error", error: errorText(e) });
        const keyHost = e.via ? latest.current.hosts.find((h) => h.id === e.via) : via;
        return setAsk({ kind: "changed", tunnel: t, expected: e.expected, actual: e.actual, host: keyHost ? `${keyHost.address}:${keyHost.port}` : "", keyHostId: e.via ?? t.viaHostId, trusted: answer.trust });
      }
      patch(t.id, { state: "error", error: errorText(e) });
    }
  };

  const toggle = (id: string) => {
    const t = latest.current.tunnels.find((x) => x.id === id);
    if (!t) return;
    if (t.state === "off" || t.state === "error") {
      // Mark as starting right away so a second click before re-render reads it as "cancel".
      latest.current = { ...latest.current, tunnels: latest.current.tunnels.map((x) => (x.id === id ? { ...x, state: "starting" } : x)) };
      void start(t);
    } else {
      latest.current = { ...latest.current, tunnels: latest.current.tunnels.map((x) => (x.id === id ? { ...x, state: "off" } : x)) };
      stop(t);
    }
  };

  const remove = (t: Tunnel) => {
    stop(t);
    update((w) => ({ ...w, tunnels: w.tunnels.filter((x) => x.id !== t.id) }));
    toast({ text: tx.tunnels.removed(t.name), action: { label: tx.tunnels.restore, run: () => update((w) => ({ ...w, tunnels: [...w.tunnels, { ...t, state: "off", error: undefined }] })) } });
  };

  const copy = async (t: Tunnel) => {
    const text = endpoint(t);
    toast((await copyText(text)) ? { text: tx.tunnels.copied(text), tone: "success" } : { text: tx.common.copyFailed, tone: "error" });
  };

  if (hosts.length === 0) {
    return (
      <div className="page">
        <PageHeader title={tx.tunnels.title} />
        <Empty icon={ArrowLeftRight} title={tx.tunnels.needHost} action={<Button variant="primary" icon={Plus} onClick={() => navigate("hosts")}>{tx.tunnels.toHosts}</Button>} />
      </div>
    );
  }

  return (
    <div className="page">
      <PageHeader title={tx.tunnels.title} count={tunnels.length || undefined}>
        {canCreate && <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>{tx.tunnels.createTunnel}</Button>}
      </PageHeader>

      {tunnels.length === 0 && <Empty icon={ArrowLeftRight} title={tx.tunnels.empty} action={canCreate ? <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>{tx.tunnels.createTunnel}</Button> : undefined} />}

      <div className="rows">
        {tunnels.map((t) => {
          const via = ws.hosts.find((h) => h.id === t.viaHostId);
          const canEdit = atLeast(t.access, "edit");
          return (
            <div className={`item-row tunnel-row ${t.state}`} key={t.id} onDoubleClick={() => canEdit && setEditing(t)}>
              <Switch label={t.state === "on" ? tx.tunnels.close : tx.tunnels.open} checked={t.state === "on" || t.state === "starting"} busy={t.state === "starting"} onChange={() => toggle(t.id)} />
              <div className="item-main">
                <span className="item-title">{t.name}</span>
                <Route nodes={routeOf(t.kind, t.bindPort, via?.label ?? "?", t.target)} />
              </div>

              <div className="tunnel-status">
                {t.state === "starting" && (
                  <span className="muted"><Loader2 size={13} className="spin" /> {tx.tunnels.opening}</span>
                )}
                {t.state === "on" && (
                  <>
                    <span className="ok-text">{tx.tunnels.isOpen}</span>
                    {!!t.connections && <span className="muted small">{tx.tunnels.connections(t.connections)}</span>}
                    <Button size="sm" variant="ghost" icon={Copy} onClick={() => copy(t)}>{endpoint(t)}</Button>
                  </>
                )}
                {t.state === "error" && (
                  <>
                    <span className="err-text">{t.error}</span>
                    <Button size="sm" icon={RotateCw} onClick={() => start(t)}>{tx.common.retry}</Button>
                  </>
                )}
              </div>

              {canEdit ? (
                <Menu
                  trigger={(open, isOpen) => <IconButton icon={Ellipsis} label={tx.tunnels.more} className={isOpen ? "pressed" : ""} onClick={open} />}
                  items={[
                    { label: tx.common.edit, icon: Pencil, onSelect: () => setEditing(t) },
                    { label: tx.common.delete, icon: Trash2, danger: true, onSelect: () => remove(t) },
                  ]}
                />
              ) : (
                <span className="icon-spacer" />
              )}
            </div>
          );
        })}
      </div>

      {ask?.kind === "secret" && (
        <SecretDialog
          ask={ask}
          onClose={() => setAsk(null)}
          onAnswer={(secret, remember) => {
            setAsk(null);
            void start(ask.tunnel, { secret, secretKind: ask.secret, remember, trust: ask.trust });
          }}
        />
      )}
      {ask?.kind === "trust" && (
        <TrustDialog
          ask={ask}
          onClose={() => setAsk(null)}
          onTrust={() => {
            setAsk(null);
            void start(ask.tunnel, { trust: trustMore(ask.trusted, ask.fingerprint) });
          }}
        />
      )}

      {ask?.kind === "changed" && (
        <ChangedKeyDialog
          ask={ask}
          onClose={() => setAsk(null)}
          onReplace={async () => {
            setAsk(null);
            try {
              await api.hostForgetKey(ask.tunnel.vaultId, ask.keyHostId);
              void start(ask.tunnel, { trust: trustMore(ask.trusted, ask.actual) });
            } catch (e) {
              toast({ text: errorText(e), tone: "error" });
            }
          }}
        />
      )}

      {editing && (
        <TunnelDialog
          tunnel={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={(t, wasOn) => wasOn && start(t)}
        />
      )}
    </div>
  );
}

/* ---------- what a login asks for ---------- */

function SecretDialog({ ask, onClose, onAnswer }: { ask: Extract<Ask, { kind: "secret" }>; onClose: () => void; onAnswer: (secret: string, remember: boolean) => void }) {
  const tx = useT();
  const [value, setValue] = useState("");
  const [remember, setRemember] = useState(true);
  return (
    <Modal
      title={tx.tunnels.secret.title(ask.tunnel.name)}
      onClose={onClose}
      onSubmit={() => value && onAnswer(value, remember)}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{tx.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!value}>{tx.tunnels.open}</Button>
        </>
      }
    >
      <Field label={ask.text} error={ask.error}>
        <PasswordInput value={value} onChange={setValue} invalid={!!ask.error} autoFocus />
      </Field>
      <Checkbox checked={remember} onChange={setRemember}>{tx.tunnels.secret.saveToVault}</Checkbox>
    </Modal>
  );
}

function TrustDialog({ ask, onClose, onTrust }: { ask: Extract<Ask, { kind: "trust" }>; onClose: () => void; onTrust: () => void }) {
  const tx = useT();
  return (
    <Modal
      title={tx.tunnels.trust.title}
      onClose={onClose}
      onSubmit={onTrust}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{tx.common.cancel}</Button>
          <Button type="submit" variant="primary">{tx.tunnels.trust.trustAndOpen}</Button>
        </>
      }
    >
      <div className="trust-card">
        <ShieldQuestion size={18} />
        <div>
          <div className="strong mono">{ask.host}</div>
          <div className="fp mono">{ask.fingerprint}</div>
        </div>
      </div>
    </Modal>
  );
}

/** What the login prompt asks for, in words. */
function secretLabel(kind: SecretKind, via?: Host) {
  const tx = tr();
  return kind === "passphrase" ? tx.tunnels.secret.passphrase : via ? tx.tunnels.secret.passwordFor(`${via.user}@${via.address}`) : tx.common.password;
}

function ChangedKeyDialog({ ask, onClose, onReplace }: { ask: Extract<Ask, { kind: "changed" }>; onClose: () => void; onReplace: () => void }) {
  const tx = useT();
  return (
    <Modal
      title={tx.tunnels.changed.title}
      onClose={onClose}
      footer={
        <>
          <Button type="button" variant="ghost" className="danger-text" onClick={onReplace}>{tx.tunnels.changed.replace}</Button>
          <Button type="button" variant="primary" autoFocus onClick={onClose}>{tx.tunnels.changed.keep}</Button>
        </>
      }
    >
      <div className="hostkey-host mono">{ask.host}</div>
      <dl className="hostkey-fp">
        <dt>{tx.tunnels.changed.was}</dt>
        <dd className="mono">{ask.expected}</dd>
        <dt>{tx.tunnels.changed.now}</dt>
        <dd className="mono">{ask.actual}</dd>
      </dl>
    </Modal>
  );
}

/* ---------- create / edit ---------- */

const kinds: { value: Tunnel["kind"]; icon: LucideIcon }[] = [
  { value: "local", icon: Laptop },
  { value: "remote", icon: Server },
  { value: "dynamic", icon: Globe },
];

const portOk = (p: string) => Number(p) > 0 && Number(p) < 65536;
const onlyDigits = (s: string) => s.replace(/\D/g, "").slice(0, 5);

function splitTarget(target?: string): [string, string] {
  const m = target?.match(/^(.*):(\d+)$/);
  return m ? [m[1]!, m[2]!] : ["localhost", ""];
}

function TunnelDialog({ tunnel, onClose, onSaved }: { tunnel: Tunnel | null; onClose: () => void; onSaved: (t: Tunnel, wasOn: boolean) => void }) {
  const tx = useT();
  const { ws, vault, update, toast } = useStore();
  const hosts = ws.hosts.filter((h) => h.vaultId === vault.id);
  const [initHost, initPort] = splitTarget(tunnel?.target);

  const [kind, setKind] = useState<Tunnel["kind"]>(tunnel?.kind ?? "local");
  const [via, setVia] = useState(tunnel?.viaHostId ?? hosts[0]?.id ?? "");
  const [inPort, setInPort] = useState(tunnel ? String(tunnel.bindPort) : "");
  const [inTouched, setInTouched] = useState(!!tunnel);
  const [outHost, setOutHost] = useState(initHost);
  const [outPort, setOutPort] = useState(initPort);
  const [name, setName] = useState(tunnel?.name ?? "");
  const [showErrors, setShowErrors] = useState(false);

  const viaLabel = hosts.find((h) => h.id === via)?.label ?? tx.tunnels.dialog.serverFallback;
  // For "service → here" the local port follows the service port until edited by hand.
  const entryPort = kind === "local" && !inTouched ? outPort : inPort;
  const outHostOk = /^[^\s:]+$/.test(outHost.trim());
  const errors = {
    inPort: !portOk(entryPort),
    outHost: kind !== "dynamic" && !outHostOk,
    outPort: kind !== "dynamic" && !portOk(outPort),
  };
  const autoName =
    kind === "dynamic" ? tx.tunnels.dialog.proxyVia(viaLabel) : kind === "remote" ? tx.tunnels.dialog.onServer(`localhost:${outPort || "…"}`, viaLabel) : `${outHost.trim() || "…"}:${outPort || "…"}`;

  const pickKind = (k: Tunnel["kind"]) => {
    if (k === kind) return;
    setKind(k);
    if (tunnel) return;
    setShowErrors(false);
    setInTouched(k !== "local");
    setInPort(k === "remote" ? "8080" : k === "dynamic" ? "1080" : "");
    setOutHost("localhost");
    setOutPort(k === "remote" ? "3000" : "");
  };

  const submit = () => {
    setShowErrors(true);
    if (errors.inPort || errors.outHost || errors.outPort || !via) return;
    const wasOn = tunnel?.state === "on";
    const saved: Tunnel = {
      id: tunnel?.id ?? uid("t"),
      vaultId: vault.id,
      name: name.trim() || autoName,
      kind,
      bindPort: Number(entryPort),
      viaHostId: via,
      target: kind === "dynamic" ? undefined : `${outHost.trim()}:${outPort}`,
      state: "off",
    };
    update((w) => ({ ...w, tunnels: tunnel ? w.tunnels.map((t) => (t.id === tunnel.id ? saved : t)) : [...w.tunnels, saved] }));
    toast({ text: tunnel ? tx.tunnels.dialog.saved : tx.tunnels.dialog.created(saved.name), tone: "success" });
    onSaved(saved, wasOn);
    onClose();
  };

  const err = (bad: boolean) => (showErrors && bad ? "invalid" : "");

  const portInput = (value: string, set: (v: string) => void, placeholder: string, bad: boolean, onEdit?: () => void) => (
    <input
      className={`input mono endpoint-port ${err(bad)}`}
      value={value}
      inputMode="numeric"
      placeholder={placeholder}
      aria-label={tx.tunnels.dialog.port}
      onChange={(e) => {
        onEdit?.();
        set(onlyDigits(e.target.value));
      }}
    />
  );

  const here = { icon: Laptop, title: tx.tunnels.route.thisComputer };
  const server = { icon: Server, title: viaLabel };

  const entry =
    kind === "remote"
      ? { where: server, prefix: viaLabel, port: portInput(inPort, setInPort, "8080", errors.inPort) }
      : { where: here, prefix: "localhost", port: portInput(entryPort, setInPort, kind === "dynamic" ? "1080" : "5432", errors.inPort, () => setInTouched(true)) };

  const exitWhere = kind === "remote" ? here : server;

  return (
    <Modal
      title={tunnel ? tx.tunnels.dialog.title : tx.tunnels.dialog.newTitle}
      width={680}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{tx.common.cancel}</Button>
          <Button type="submit" variant="primary">{tunnel ? tx.common.save : tx.common.create}</Button>
        </>
      }
    >
      <div className="kind-cards" role="radiogroup">
        {kinds.map((k) => (
          <button type="button" role="radio" aria-checked={kind === k.value} key={k.value} className={`kind-card ${kind === k.value ? "on" : ""}`} onClick={() => pickKind(k.value)}>
            <k.icon size={16} />
            <span className="kind-title">{tx.tunnels.kinds[k.value]}</span>
          </button>
        ))}
      </div>

      <Field label={tx.tunnels.dialog.server}>
        <HostSelect hosts={hosts} value={via} onChange={setVia} />
      </Field>

      <div className="endpoints">
        <div className="endpoint">
          <span className="endpoint-where"><entry.where.icon size={14} /> {entry.where.title}</span>
          <div className="endpoint-addr">
            <span className="endpoint-fixed mono">{entry.prefix}</span>
            <span className="colon">:</span>
            {entry.port}
          </div>
        </div>

        <ArrowRight size={18} className="endpoints-arrow" />

        <div className="endpoint">
          <span className="endpoint-where"><exitWhere.icon size={14} /> {exitWhere.title}</span>
          <div className="endpoint-addr">
            {kind === "dynamic" ? (
              <span className="endpoint-fixed muted">{tx.tunnels.dialog.anyTarget}</span>
            ) : (
              <>
                <input
                  className={`input mono ${err(errors.outHost)}`}
                  value={outHost}
                  onChange={(e) => setOutHost(e.target.value)}
                  placeholder="localhost"
                  aria-label={tx.tunnels.dialog.address}
                  spellCheck={false}
                />
                <span className="colon">:</span>
                {portInput(outPort, setOutPort, kind === "remote" ? "3000" : "5432", errors.outPort)}
              </>
            )}
          </div>
        </div>
      </div>

      <Field label={tx.tunnels.dialog.name}>
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder={autoName} />
      </Field>
    </Modal>
  );
}


function HostSelect({ hosts, value, onChange }: { hosts: Host[]; value: string; onChange: (id: string) => void }) {
  const tx = useT();
  return (
    <select className="input" value={value} onChange={(e) => onChange(e.target.value)}>
      {hosts.map((h) => (
        <option key={h.id} value={h.id}>
          {h.label}
          {h.online === false ? ` (${tx.tunnels.dialog.unreachable})` : ""}
        </option>
      ))}
    </select>
  );
}
