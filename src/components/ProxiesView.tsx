import { useRef, useState } from "react";
import { Activity, ClipboardPaste, Copy, Ellipsis, FileUp, Globe, Loader2, Network, Pencil, Plus, Shield, Trash2, Waypoints, type LucideIcon } from "lucide-react";
import { api, errorText, isCoreError, type ParsedProxy } from "../lib/api";
import { sleep, uid, useStore } from "../lib/store";
import type { ProxyKind, ProxySecret, ProxyType, SavedProxy } from "../lib/types";
import { atLeast, writable } from "../lib/teams";
import { Button, Checkbox, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented } from "./ui";
import { tr, useT } from "../i18n";

export const proxyIcon: Record<ProxyType, LucideIcon> = { socks5: Network, http: Globe, wireguard: Shield, vless: Waypoints };

/** Where a proxy is and how it is reached, in a line. */
export function proxySummary(k: ProxyKind): string {
  switch (k.type) {
    case "socks5":
    case "http":
      return `${k.user ? `${k.user}@` : ""}${k.address}:${k.port}`;
    case "wireguard":
      return k.endpoint;
    case "vless": {
      const security = k.security === "reality" ? "REALITY" : k.security === "tls" ? "TLS" : "";
      return [`${k.address}:${k.port}`, security, k.transport === "xhttp" ? "XHTTP" : ""].filter(Boolean).join(" · ");
    }
  }
}

/** The secrets saved once the typed ones are in the vault. */
export function savedAfter(saved: ProxySecret[], typed: Partial<Record<ProxySecret, string>>): ProxySecret[] {
  const out = new Set(saved);
  for (const [k, v] of Object.entries(typed) as [ProxySecret, string][]) {
    if (v) out.add(k);
    else out.delete(k);
  }
  return [...out];
}

type Check = { state: "checking" } | { state: "ok"; ms: number } | { state: "error"; text: string };

/** Tries a proxy as it is now, typed secrets included. */
async function runCheck(p: SavedProxy): Promise<Check> {
  try {
    if (!api.real) {
      await sleep(700);
      return { state: "ok", ms: 40 + Math.round(Math.random() * 80) };
    }
    const { ms } = await api.proxyCheck(p);
    return { state: "ok", ms };
  } catch (e) {
    return { state: "error", text: errorText(e) };
  }
}

function CheckResult({ check }: { check?: Check }) {
  const tx = useT().proxies;
  if (!check) return null;
  if (check.state === "checking") return <span className="muted"><Loader2 size={13} className="spin" /> {tx.checking}</span>;
  if (check.state === "ok") return <span className="ok-text">{tx.works(check.ms)}</span>;
  return <span className="err-text">{check.text}</span>;
}

/* ---------- page ---------- */

export function ProxiesView() {
  const tx = useT();
  const { ws, vault, update, toast } = useStore();
  const canCreate = writable(vault);
  const [editing, setEditing] = useState<SavedProxy | "new" | null>(null);
  const [parsed, setParsed] = useState<ParsedProxy | null>(null);
  const [checks, setChecks] = useState<Record<string, Check>>({});
  const fileRef = useRef<HTMLInputElement>(null);

  /** A link or a config becomes a new proxy to look over and save. */
  const read = async (text: string) => {
    try {
      setParsed(await api.proxyParse(text));
      setEditing("new");
    } catch (e) {
      const unsupported = isCoreError(e) && e.kind === "proxy" && e.reason.startsWith("unsupported:");
      toast({ text: unsupported ? errorText(e) : tx.proxies.notALink, tone: "error" });
    }
  };
  const paste = async () => read(await api.clipboardRead().catch(() => ""));
  const readFile = async (f: File | undefined) => {
    if (f && f.size < 64 * 1024) await read(await f.text());
    else if (f) toast({ text: tx.proxies.notALink, tone: "error" });
  };

  const copy = async (p: SavedProxy) => {
    try {
      const { text } = await api.proxyExport(p);
      toast((await api.clipboardWrite(text)) ? { text: tx.proxies.copied, tone: "success" } : { text: tx.common.copyFailed, tone: "error" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const proxies = ws.proxies.filter((p) => p.vaultId === vault.id);

  const check = async (p: SavedProxy) => {
    setChecks((c) => ({ ...c, [p.id]: { state: "checking" } }));
    const result = await runCheck(p);
    setChecks((c) => ({ ...c, [p.id]: result }));
  };

  const remove = (p: SavedProxy) => {
    const users = ws.hosts.filter((h) => h.proxyId === p.id).map((h) => h.id);
    update((w) => ({ ...w, proxies: w.proxies.filter((x) => x.id !== p.id), hosts: w.hosts.map((h) => (h.proxyId === p.id ? { ...h, proxyId: null } : h)) }));
    toast({
      text: tx.proxies.removed(p.name),
      action: {
        label: tx.proxies.restore,
        run: () => update((w) => ({ ...w, proxies: [...w.proxies, p], hosts: w.hosts.map((h) => (users.includes(h.id) ? { ...h, proxyId: p.id } : h)) })),
      },
    });
  };

  return (
    <div
      className="page"
      onDragOver={(e) => canCreate && e.dataTransfer.types.includes("Files") && e.preventDefault()}
      onDrop={(e) => {
        if (!canCreate || !e.dataTransfer.files.length) return;
        e.preventDefault();
        void readFile(e.dataTransfer.files[0]);
      }}
    >
      <PageHeader title={tx.proxies.title} count={proxies.length || undefined}>
        {canCreate && (
          <>
            <Button icon={ClipboardPaste} onClick={() => void paste()}>{tx.proxies.paste}</Button>
            <Button icon={FileUp} onClick={() => fileRef.current?.click()}>{tx.proxies.fromFile}</Button>
            <input
              ref={fileRef}
              type="file"
              accept=".conf,.txt"
              hidden
              onChange={(e) => {
                void readFile(e.target.files?.[0]);
                e.target.value = "";
              }}
            />
            <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>{tx.proxies.add}</Button>
          </>
        )}
      </PageHeader>

      {proxies.length === 0 && <Empty icon={Shield} title={tx.proxies.empty} action={canCreate ? <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>{tx.proxies.add}</Button> : undefined} />}

      <div className="rows">
        {proxies.map((p) => {
          const Icon = proxyIcon[p.kind.type];
          const canEdit = atLeast(p.access, "edit");
          const used = ws.hosts.filter((h) => h.proxyId === p.id).length;
          return (
            <div className="item-row proxy-row" key={p.id} onDoubleClick={() => canEdit && setEditing(p)}>
              <span className="item-icon"><Icon size={15} /></span>
              <div className="item-main">
                <span className="item-title">
                  {p.name}
                  <span className="chip">{tx.proxies.types[p.kind.type]}</span>
                </span>
                <span className="item-sub mono">{proxySummary(p.kind)}</span>
              </div>
              <div className="tunnel-status">
                <CheckResult check={checks[p.id]} />
                {checks[p.id]?.state !== "checking" && <Button size="sm" variant="ghost" icon={Activity} onClick={() => void check(p)}>{tx.proxies.check}</Button>}
              </div>
              <span className="item-meta proxy-used">{used > 0 ? tx.proxies.hosts(used) : ""}</span>
              {canEdit ? (
                <Menu
                  trigger={(open, isOpen) => <IconButton icon={Ellipsis} label={tx.proxies.more} className={isOpen ? "pressed" : ""} onClick={open} />}
                  items={[
                    { label: tx.common.edit, icon: Pencil, onSelect: () => setEditing(p) },
                    ...(atLeast(p.access, "secret") && (p.kind.type === "vless" || p.kind.type === "wireguard")
                      ? [{ label: p.kind.type === "vless" ? tx.proxies.copyLink : tx.proxies.copyConfig, icon: Copy, onSelect: () => void copy(p) }]
                      : []),
                    { label: tx.common.delete, icon: Trash2, danger: true, onSelect: () => remove(p) },
                  ]}
                />
              ) : (
                <span className="icon-spacer" />
              )}
            </div>
          );
        })}
      </div>

      {editing && (
        <ProxyDialog
          proxy={editing === "new" ? null : editing}
          parsed={editing === "new" ? parsed : null}
          onClose={() => {
            setEditing(null);
            setParsed(null);
          }}
        />
      )}
    </div>
  );
}

/* ---------- create / edit ---------- */

const types: ProxyType[] = ["socks5", "http", "wireguard", "vless"];
const modes = ["auto", "packet-up", "stream-up", "stream-one"];

const portOk = (p: string) => /^\d+$/.test(p) && Number(p) > 0 && Number(p) < 65536;
const digits = (s: string) => s.replace(/\D/g, "").slice(0, 5);
const list = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
const wgKey = (s: string) => /^[A-Za-z0-9+/]{43}=$/.test(s.trim());
const cidr = (s: string) => /^[0-9a-fA-F:.]+(\/\d{1,3})?$/.test(s);
const uuidOk = (s: string) => /^[0-9a-fA-F]{8}-?[0-9a-fA-F]{4}-?[0-9a-fA-F]{4}-?[0-9a-fA-F]{4}-?[0-9a-fA-F]{12}$/.test(s.trim()) || (s.trim().length > 0 && s.trim().length <= 30);

/** Everything the dialog edits, as text. */
interface Draft {
  type: ProxyType;
  name: string;
  address: string;
  port: string;
  user: string;
  endpoint: string;
  peerKey: string;
  addresses: string;
  dns: string;
  security: "none" | "tls" | "reality";
  sni: string;
  publicKey: string;
  shortId: string;
  transport: "tcp" | "xhttp";
  vision: boolean;
  path: string;
  host: string;
  mode: string;
  secrets: Partial<Record<ProxySecret, string>>;
}

const defaultPort: Record<ProxyType, string> = { socks5: "1080", http: "3128", wireguard: "51820", vless: "443" };

function draftOf(p: SavedProxy | null): Draft {
  const d: Draft = {
    type: p?.kind.type ?? "socks5",
    name: p?.name ?? "",
    address: "",
    port: defaultPort[p?.kind.type ?? "socks5"],
    user: "",
    endpoint: "",
    peerKey: "",
    addresses: "",
    dns: "",
    security: "reality",
    sni: "",
    publicKey: "",
    shortId: "",
    transport: "tcp",
    vision: true,
    path: "",
    host: "",
    mode: "auto",
    secrets: {},
  };
  const k = p?.kind;
  if (!k) return d;
  if (k.type === "socks5" || k.type === "http") return { ...d, address: k.address, port: String(k.port), user: k.user };
  if (k.type === "wireguard") return { ...d, endpoint: k.endpoint, peerKey: k.publicKey, addresses: k.addresses.join(", "), dns: k.dns.join(", ") };
  if (k.type !== "vless") return d;
  return {
    ...d,
    address: k.address,
    port: String(k.port),
    security: k.security,
    sni: k.sni,
    publicKey: k.publicKey,
    shortId: k.shortId,
    transport: k.transport,
    vision: k.flow === "xtls-rprx-vision",
    path: k.path,
    host: k.host,
    mode: k.mode || "auto",
  };
}

/** The proxy a draft describes, keeping what the dialog doesn't show (MTU, link parameters). */
function kindOf(d: Draft, old: ProxyKind | undefined): ProxyKind {
  switch (d.type) {
    case "socks5":
    case "http":
      return { type: d.type, address: d.address.trim(), port: Number(d.port), user: d.user.trim() };
    case "wireguard": {
      const prev = old?.type === "wireguard" ? old : undefined;
      return { type: "wireguard", endpoint: d.endpoint.trim(), publicKey: d.peerKey.trim(), addresses: list(d.addresses), dns: list(d.dns), mtu: prev?.mtu ?? null, allowedIps: prev?.allowedIps ?? ["0.0.0.0/0", "::/0"], keepalive: prev?.keepalive ?? null };
    }
    case "vless": {
      const prev = old?.type === "vless" ? old : undefined;
      return {
        type: "vless",
        address: d.address.trim(),
        port: Number(d.port),
        security: d.security,
        sni: d.security === "none" ? "" : d.sni.trim(),
        alpn: prev?.alpn ?? [],
        publicKey: d.security === "reality" ? d.publicKey.trim() : "",
        shortId: d.security === "reality" ? d.shortId.trim() : "",
        pinnedCert: d.security === "tls" ? prev?.pinnedCert ?? "" : "",
        transport: d.transport,
        // Vision needs TLS or REALITY under it, and doesn't go with XHTTP.
        flow: d.transport === "tcp" && d.security !== "none" && d.vision ? "xtls-rprx-vision" : "",
        path: d.transport === "xhttp" ? d.path.trim() : "",
        host: d.transport === "xhttp" ? d.host.trim() : "",
        mode: d.transport === "xhttp" ? d.mode : "",
        extra: d.transport === "xhttp" ? prev?.extra ?? "" : "",
        params: prev?.params ?? {},
      };
    }
  }
}

/** The secrets a type of proxy asks for. */
const secretsOf = (type: ProxyType): ProxySecret[] => (type === "wireguard" ? ["privateKey", "presharedKey"] : type === "vless" ? ["uuid"] : ["password"]);

export function ProxyDialog({ proxy, parsed, onClose, onCreated }: { proxy: SavedProxy | null; parsed?: ParsedProxy | null; onClose: () => void; onCreated?: (id: string) => void }) {
  const tx = useT();
  const t = tx.proxies.dialog;
  const e = tx.proxies.errors;
  const { vault, update, toast } = useStore();
  // A pasted proxy: its secrets are as if typed in.
  const [d, setD] = useState(() => (parsed ? { ...draftOf({ id: "", vaultId: "", name: parsed.name, kind: parsed.kind, saved: [] }), secrets: parsed.secrets } : draftOf(proxy)));
  const [showErrors, setShowErrors] = useState(false);
  const [check, setCheck] = useState<Check>();
  const set = (patch: Partial<Draft>) => {
    setD((x) => ({ ...x, ...patch }));
    setCheck(undefined);
  };
  const setSecret = (k: ProxySecret, v: string) => set({ secrets: { ...d.secrets, [k]: v } });

  // A secret is there if it is typed now, or saved and the type is the same as when it was saved.
  const has = (k: ProxySecret) => !!d.secrets[k] || (!!proxy?.saved.includes(k) && proxy.kind.type === d.type);
  const savedHint = (k: ProxySecret) => (proxy?.saved.includes(k) && proxy.kind.type === d.type ? t.saved : "");

  const errors: Record<string, string | null> = {};
  if (d.type === "socks5" || d.type === "http" || d.type === "vless") {
    errors.address = d.address.trim() ? null : e.required;
    errors.port = portOk(d.port) ? null : e.port;
  }
  if (d.type === "wireguard") {
    errors.endpoint = /^\S+:\d{1,5}$/.test(d.endpoint.trim()) ? null : e.endpoint;
    errors.peerKey = wgKey(d.peerKey) ? null : e.key;
    errors.privateKey = d.secrets.privateKey ? (wgKey(d.secrets.privateKey) ? null : e.key) : has("privateKey") ? null : e.required;
    errors.presharedKey = d.secrets.presharedKey && !wgKey(d.secrets.presharedKey) ? e.key : null;
    errors.addresses = list(d.addresses).length && list(d.addresses).every(cidr) ? null : e.address;
  }
  if (d.type === "vless") {
    errors.uuid = d.secrets.uuid ? (uuidOk(d.secrets.uuid) ? null : e.uuid) : has("uuid") ? null : e.required;
    errors.sni = d.security === "reality" && !d.sni.trim() ? e.required : null;
    errors.publicKey = d.security === "reality" && !d.publicKey.trim() ? e.required : null;
  }
  const invalid = Object.values(errors).some(Boolean);
  const err = (k: string) => (showErrors ? errors[k] ?? null : null);
  const cls = (k: string) => (err(k) ? "invalid" : "");

  const kind = kindOf(d, proxy?.kind ?? parsed?.kind);
  const where = d.type === "wireguard" ? d.endpoint.trim() : d.address.trim();
  const autoName = where ? proxySummary(kind).split(" · ")[0]! : tx.proxies.types[d.type];

  /** The proxy as saved, with typed secrets (only those its type uses). */
  const result = (): SavedProxy => {
    const typed = Object.fromEntries(secretsOf(d.type).filter((k) => d.secrets[k] !== undefined && (d.secrets[k] !== "" || has(k))).map((k) => [k, d.secrets[k] ?? ""]));
    // A secret saved for another type of proxy goes away with it.
    const saved = proxy && proxy.kind.type === d.type ? proxy.saved : [];
    return { id: proxy?.id ?? uid("p"), vaultId: vault.id, name: d.name.trim() || autoName, kind, saved, newSecrets: Object.keys(typed).length ? typed : undefined, access: proxy?.access };
  };

  const submit = () => {
    setShowErrors(true);
    if (invalid) return;
    const p = result();
    update((w) => ({ ...w, proxies: proxy ? w.proxies.map((x) => (x.id === proxy.id ? p : x)) : [...w.proxies, p] }));
    toast({ text: proxy ? t.savedToast : t.created(p.name), tone: "success" });
    if (!proxy) onCreated?.(p.id);
    onClose();
  };

  const tryIt = async () => {
    setShowErrors(true);
    if (invalid) return;
    setCheck({ state: "checking" });
    setCheck(await runCheck(result()));
  };

  const pickType = (type: ProxyType) => {
    if (type === d.type) return;
    set({ type, port: d.port === defaultPort[d.type] || !d.port ? defaultPort[type] : d.port });
    setShowErrors(false);
  };

  const secretField = (k: ProxySecret, label: string, optional = false) => (
    <Field label={label} error={err(k)}>
      <PasswordInput value={d.secrets[k] ?? ""} onChange={(v) => setSecret(k, v)} placeholder={savedHint(k) || (optional ? t.optional : "")} invalid={!!err(k)} />
    </Field>
  );

  const addressRow = (
    <div className="row">
      <Field label={t.address} error={err("address")}>
        <input className={`input mono ${cls("address")}`} value={d.address} onChange={(x) => set({ address: x.target.value })} placeholder={d.type === "vless" ? "vpn.example.com" : "proxy.example.com"} autoComplete="off" spellCheck={false} />
      </Field>
      <Field label={t.port} error={err("port")}>
        <input className={`input mono port ${cls("port")}`} value={d.port} inputMode="numeric" onChange={(x) => set({ port: digits(x.target.value) })} />
      </Field>
    </div>
  );

  return (
    <Modal
      title={proxy ? t.title : t.newTitle}
      width={560}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <div className="modal-foot-left">
            {check?.state !== "checking" && <Button type="button" variant="ghost" icon={Activity} onClick={() => void tryIt()}>{tx.proxies.check}</Button>}
            <CheckResult check={check} />
          </div>
          <Button type="button" variant="ghost" onClick={onClose}>{tx.common.cancel}</Button>
          <Button type="submit" variant="primary">{proxy ? tx.common.save : tx.common.create}</Button>
        </>
      }
    >
      <div className="kind-cards proxy-kinds" role="radiogroup">
        {types.map((type) => {
          const Icon = proxyIcon[type];
          return (
            <button type="button" role="radio" aria-checked={d.type === type} key={type} className={`kind-card ${d.type === type ? "on" : ""}`} onClick={() => pickType(type)}>
              <Icon size={16} />
              <span className="kind-title">{tx.proxies.types[type]}</span>
            </button>
          );
        })}
      </div>

      {(d.type === "socks5" || d.type === "http") && (
        <>
          {addressRow}
          <div className="row">
            <Field label={t.user}>
              <input className="input mono" value={d.user} onChange={(x) => set({ user: x.target.value })} placeholder={t.noLogin} autoComplete="off" spellCheck={false} />
            </Field>
            {d.user.trim() ? secretField("password", t.password) : <div className="field" />}
          </div>
        </>
      )}

      {d.type === "wireguard" && (
        <>
          <Field label={t.endpoint} error={err("endpoint")}>
            <input className={`input mono ${cls("endpoint")}`} value={d.endpoint} onChange={(x) => set({ endpoint: x.target.value })} placeholder="vpn.example.com:51820" autoComplete="off" spellCheck={false} />
          </Field>
          <Field label={t.peerKey} error={err("peerKey")}>
            <input className={`input mono ${cls("peerKey")}`} value={d.peerKey} onChange={(x) => set({ peerKey: x.target.value })} autoComplete="off" spellCheck={false} />
          </Field>
          <div className="row">
            {secretField("privateKey", t.privateKey)}
            {secretField("presharedKey", t.presharedKey, true)}
          </div>
          <div className="row">
            <Field label={t.addresses} error={err("addresses")}>
              <input className={`input mono ${cls("addresses")}`} value={d.addresses} onChange={(x) => set({ addresses: x.target.value })} placeholder="10.0.0.2/32" autoComplete="off" spellCheck={false} />
            </Field>
            <Field label={t.dns}>
              <input className="input mono" value={d.dns} onChange={(x) => set({ dns: x.target.value })} placeholder={t.optional} autoComplete="off" spellCheck={false} />
            </Field>
          </div>
        </>
      )}

      {d.type === "vless" && (
        <>
          {addressRow}
          {secretField("uuid", t.uuid)}
          <Field label={t.security}>
            <Segmented
              value={d.security}
              onChange={(security) => set({ security })}
              options={[
                { value: "reality", label: "REALITY" },
                { value: "tls", label: "TLS" },
                { value: "none", label: t.none },
              ]}
            />
          </Field>
          {d.security !== "none" && (
            <Field label={t.sni} error={err("sni")}>
              <input className={`input mono ${cls("sni")}`} value={d.sni} onChange={(x) => set({ sni: x.target.value })} placeholder={d.security === "tls" ? d.address || "example.com" : "www.microsoft.com"} autoComplete="off" spellCheck={false} />
            </Field>
          )}
          {d.security === "reality" && (
            <div className="row">
              <Field label={t.publicKey} error={err("publicKey")}>
                <input className={`input mono ${cls("publicKey")}`} value={d.publicKey} onChange={(x) => set({ publicKey: x.target.value })} autoComplete="off" spellCheck={false} />
              </Field>
              <Field label={t.shortId}>
                <input className="input mono" value={d.shortId} onChange={(x) => set({ shortId: x.target.value })} placeholder={t.optional} autoComplete="off" spellCheck={false} />
              </Field>
            </div>
          )}
          <Field label={t.transport}>
            <Segmented
              value={d.transport}
              onChange={(transport) => set({ transport })}
              options={[
                { value: "tcp", label: "TCP" },
                { value: "xhttp", label: "XHTTP" },
              ]}
            />
          </Field>
          {d.transport === "tcp" && d.security !== "none" && (
            <div className="field">
              <Checkbox checked={d.vision} onChange={(vision) => set({ vision })}>{t.vision}</Checkbox>
            </div>
          )}
          {d.transport === "xhttp" && (
            <div className="row">
              <Field label={t.path}>
                <input className="input mono" value={d.path} onChange={(x) => set({ path: x.target.value })} placeholder="/" autoComplete="off" spellCheck={false} />
              </Field>
              <Field label={t.host}>
                <input className="input mono" value={d.host} onChange={(x) => set({ host: x.target.value })} placeholder={t.optional} autoComplete="off" spellCheck={false} />
              </Field>
              <Field label={t.mode}>
                <select className="input" value={d.mode} onChange={(x) => set({ mode: x.target.value })}>
                  {modes.map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              </Field>
            </div>
          )}
        </>
      )}

      <Field label={t.name}>
        <input className="input" value={d.name} onChange={(x) => set({ name: x.target.value })} placeholder={autoName} />
      </Field>
    </Modal>
  );
}

/** Text of a proxy's type for other screens. */
export const proxyTypeName = (type: ProxyType) => tr().proxies.types[type];
