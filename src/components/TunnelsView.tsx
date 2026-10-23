import { useRef, useState } from "react";
import { ArrowLeftRight, ArrowRight, Copy, Ellipsis, Globe, Laptop, Loader2, Pencil, Plus, RotateCw, Server, Trash2, type LucideIcon } from "lucide-react";
import { copyText, sleep, uid, useStore } from "../lib/store";
import type { Host, Tunnel } from "../lib/types";
import { Button, Empty, Field, IconButton, Menu, Modal, PageHeader, Switch } from "./ui";

/* ---------- route ---------- */

interface RouteNode {
  icon: LucideIcon;
  title: string;
  sub?: string;
}

function routeOf(kind: Tunnel["kind"], bindPort: number | string, via: string, target?: string): RouteNode[] {
  const port = bindPort || "…";
  const me = "Этот компьютер";
  if (kind === "local") {
    // A service on the server itself is shown as the server's own port, not as a separate "localhost" hop.
    const onServer = target?.match(/^(?:localhost|127\.0\.0\.1):(\d+)$/);
    const from = { icon: Laptop, title: me, sub: `localhost:${port}` };
    if (onServer) return [from, { icon: Server, title: via || "…", sub: `порт ${onServer[1]}` }];
    return [from, { icon: Server, title: via || "…" }, { icon: Server, title: target || "…" }];
  }
  if (kind === "remote") return [{ icon: Server, title: via || "…", sub: `порт ${port}` }, { icon: Laptop, title: me, sub: target || "…" }];
  return [{ icon: Laptop, title: me, sub: `SOCKS5 localhost:${port}` }, { icon: Server, title: via || "…" }, { icon: Globe, title: "любой адрес" }];
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
function endpoint(t: Tunnel, via?: Host) {
  if (t.kind === "local") return `localhost:${t.bindPort}`;
  if (t.kind === "dynamic") return `socks5://localhost:${t.bindPort}`;
  return `${via?.address ?? "?"}:${t.bindPort}`;
}

/* ---------- page ---------- */

export function TunnelsView() {
  const { ws, vault, update, toast, navigate } = useStore();
  const [editing, setEditing] = useState<Tunnel | "new" | null>(null);
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
    patch(t.id, { state: "off", error: undefined });
  };

  const start = async (t: Tunnel) => {
    const token = (attempts.current.get(t.id) ?? 0) + 1;
    attempts.current.set(t.id, token);
    patch(t.id, { state: "starting", error: undefined });
    await sleep(900);
    if (attempts.current.get(t.id) !== token) return;

    const now = latest.current;
    const via = now.hosts.find((h) => h.id === t.viaHostId);
    const busy = t.kind !== "remote" && now.tunnels.find((o) => o.id !== t.id && o.kind !== "remote" && o.state === "on" && o.bindPort === t.bindPort);
    if (!via?.online) return patch(t.id, { state: "error", error: `${via?.label ?? "Сервер"} не отвечает` });
    if (busy) return patch(t.id, { state: "error", error: `Порт ${t.bindPort} занят «${busy.name}»` });
    patch(t.id, { state: "on" });
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
    toast({ text: `«${t.name}» удалён`, action: { label: "Вернуть", run: () => update((w) => ({ ...w, tunnels: [...w.tunnels, { ...t, state: "off", error: undefined }] })) } });
  };

  const copy = async (t: Tunnel) => {
    const text = endpoint(t, ws.hosts.find((h) => h.id === t.viaHostId));
    toast((await copyText(text)) ? { text: `Скопировано: ${text}`, tone: "success" } : { text: "Не удалось скопировать", tone: "error" });
  };

  if (hosts.length === 0) {
    return (
      <div className="page">
        <PageHeader title="Туннели" />
        <Empty icon={ArrowLeftRight} title="Для туннеля нужен хост" action={<Button variant="primary" icon={Plus} onClick={() => navigate("hosts")}>К хостам</Button>} />
      </div>
    );
  }

  return (
    <div className="page">
      <PageHeader title="Туннели" count={tunnels.length || undefined}>
        <Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>Создать туннель</Button>
      </PageHeader>

      {tunnels.length === 0 && <Empty icon={ArrowLeftRight} title="Нет туннелей" action={<Button variant="primary" icon={Plus} onClick={() => setEditing("new")}>Создать туннель</Button>} />}

      <div className="rows">
        {tunnels.map((t) => {
          const via = ws.hosts.find((h) => h.id === t.viaHostId);
          return (
            <div className={`item-row tunnel-row ${t.state}`} key={t.id} onDoubleClick={() => setEditing(t)}>
              <Switch label={t.state === "on" ? "Закрыть туннель" : "Открыть туннель"} checked={t.state === "on" || t.state === "starting"} busy={t.state === "starting"} onChange={() => toggle(t.id)} />
              <div className="item-main">
                <span className="item-title">{t.name}</span>
                <Route nodes={routeOf(t.kind, t.bindPort, via?.label ?? "?", t.target)} />
              </div>

              <div className="tunnel-status">
                {t.state === "starting" && (
                  <span className="muted"><Loader2 size={13} className="spin" /> Открывается</span>
                )}
                {t.state === "on" && (
                  <>
                    <span className="ok-text">Открыт</span>
                    <Button size="sm" variant="ghost" icon={Copy} onClick={() => copy(t)}>{endpoint(t, via)}</Button>
                  </>
                )}
                {t.state === "error" && (
                  <>
                    <span className="err-text">{t.error}</span>
                    <Button size="sm" icon={RotateCw} onClick={() => start(t)}>Повторить</Button>
                  </>
                )}
              </div>

              <Menu
                trigger={(open, isOpen) => <IconButton icon={Ellipsis} label="Ещё" className={isOpen ? "pressed" : ""} onClick={open} />}
                items={[
                  { label: "Изменить", icon: Pencil, onSelect: () => setEditing(t) },
                  { label: "Удалить", icon: Trash2, danger: true, onSelect: () => remove(t) },
                ]}
              />
            </div>
          );
        })}
      </div>

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

/* ---------- create / edit ---------- */

const kinds: { value: Tunnel["kind"]; icon: LucideIcon; title: string }[] = [
  { value: "local", icon: Laptop, title: "Сервис сервера у себя" },
  { value: "remote", icon: Server, title: "Свой сервис на сервере" },
  { value: "dynamic", icon: Globe, title: "Прокси через сервер" },
];

const portOk = (p: string) => Number(p) > 0 && Number(p) < 65536;
const onlyDigits = (s: string) => s.replace(/\D/g, "").slice(0, 5);

function splitTarget(target?: string): [string, string] {
  const m = target?.match(/^(.*):(\d+)$/);
  return m ? [m[1]!, m[2]!] : ["localhost", ""];
}

function TunnelDialog({ tunnel, onClose, onSaved }: { tunnel: Tunnel | null; onClose: () => void; onSaved: (t: Tunnel, wasOn: boolean) => void }) {
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

  const viaLabel = hosts.find((h) => h.id === via)?.label ?? "сервер";
  // For "service → here" the local port follows the service port until edited by hand.
  const entryPort = kind === "local" && !inTouched ? outPort : inPort;
  const outHostOk = /^[^\s:]+$/.test(outHost.trim());
  const errors = {
    inPort: !portOk(entryPort),
    outHost: kind !== "dynamic" && !outHostOk,
    outPort: kind !== "dynamic" && !portOk(outPort),
  };
  const autoName =
    kind === "dynamic" ? `Прокси через ${viaLabel}` : kind === "remote" ? `localhost:${outPort || "…"} на ${viaLabel}` : `${outHost.trim() || "…"}:${outPort || "…"}`;

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
    toast({ text: tunnel ? "Сохранено" : `«${saved.name}» создан`, tone: "success" });
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
      aria-label="Порт"
      onChange={(e) => {
        onEdit?.();
        set(onlyDigits(e.target.value));
      }}
    />
  );

  const here = { icon: Laptop, title: "Этот компьютер" };
  const server = { icon: Server, title: viaLabel };

  const entry =
    kind === "remote"
      ? { where: server, prefix: viaLabel, port: portInput(inPort, setInPort, "8080", errors.inPort) }
      : { where: here, prefix: "localhost", port: portInput(entryPort, setInPort, kind === "dynamic" ? "1080" : "5432", errors.inPort, () => setInTouched(true)) };

  const exitWhere = kind === "remote" ? here : server;

  return (
    <Modal
      title={tunnel ? "Туннель" : "Новый туннель"}
      width={680}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary">{tunnel ? "Сохранить" : "Создать"}</Button>
        </>
      }
    >
      <div className="kind-cards" role="radiogroup">
        {kinds.map((k) => (
          <button type="button" role="radio" aria-checked={kind === k.value} key={k.value} className={`kind-card ${kind === k.value ? "on" : ""}`} onClick={() => pickKind(k.value)}>
            <k.icon size={16} />
            <span className="kind-title">{k.title}</span>
          </button>
        ))}
      </div>

      <Field label="Сервер">
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
              <span className="endpoint-fixed muted">любой адрес : любой порт</span>
            ) : (
              <>
                <input
                  className={`input mono ${err(errors.outHost)}`}
                  value={outHost}
                  onChange={(e) => setOutHost(e.target.value)}
                  placeholder="localhost"
                  aria-label="Адрес"
                  spellCheck={false}
                />
                <span className="colon">:</span>
                {portInput(outPort, setOutPort, kind === "remote" ? "3000" : "5432", errors.outPort)}
              </>
            )}
          </div>
        </div>
      </div>

      <Field label="Название">
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder={autoName} />
      </Field>
    </Modal>
  );
}


function HostSelect({ hosts, value, onChange }: { hosts: Host[]; value: string; onChange: (id: string) => void }) {
  return (
    <select className="input" value={value} onChange={(e) => onChange(e.target.value)}>
      {hosts.map((h) => (
        <option key={h.id} value={h.id}>
          {h.label}
          {h.online ? "" : " (недоступен)"}
        </option>
      ))}
    </select>
  );
}
