import { useEffect, useState } from "react";
import { Bot, Check, ChevronDown, ChevronUp, Loader2 } from "lucide-react";
import { api, errorText, type AgentRequest, type AgentRun, type AgentScope, type AgentToken, type AgentTtl } from "../lib/api";
import { copyText, formatAgo, useStore } from "../lib/store";
import { Button, Checkbox, Modal, Segmented } from "./ui";
import { locale, useT } from "../i18n";

const TTLS: AgentTtl[] = ["2h", "1d", "7d", "never"];
const hostName = (h: { label: string; team: string | null }) => (h.team ? `${h.team} / ${h.label}` : h.label);

/** What an agent waits on: access for a token, or a command of a token that asks each time. */
export function AgentRequestDialog({ request, onDone }: { request: AgentRequest; onDone: () => void }) {
  return request.type === "access" ? <AccessDialog request={request} onDone={onDone} /> : <CommandDialog request={request} onDone={onDone} />;
}

function AccessDialog({ request, onDone }: { request: Extract<AgentRequest, { type: "access" }>; onDone: () => void }) {
  const t = useT().agents;
  const { toast } = useStore();
  const key = (h: { vaultId: string; hostId: string }) => `${h.vaultId}/${h.hostId}`;
  const [hosts, setHosts] = useState(() => new Set(request.hosts.map(key)));
  const [scopes, setScopes] = useState(() => new Set(request.scopes));
  const [ttl, setTtl] = useState<AgentTtl>(request.ttl);
  const [busy, setBusy] = useState(false);
  const toggle = <T,>(set: Set<T>, v: T, on: boolean) => {
    const next = new Set(set);
    if (on) next.add(v);
    else next.delete(v);
    return next;
  };

  const answer = async (allow: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      const picked = request.hosts.filter((h) => hosts.has(key(h))).map((h) => ({ vaultId: h.vaultId, hostId: h.hostId }));
      const got = await api.agentsAnswer(request.id, allow, allow ? { ttl, scopes: [...scopes], hosts: picked } : {});
      if (allow && got) {
        toast({
          text: t.access.granted(request.agent),
          tone: "success",
          action: { label: t.access.copyToken, run: () => void copyText(got.token).then((ok) => ok && toast({ text: t.access.tokenCopied, tone: "success" })) },
        });
      }
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    onDone();
  };

  const canAllow = hosts.size > 0 && scopes.size > 0;
  return (
    <Modal
      title={t.access.title(request.agent)}
      width={480}
      onClose={() => void answer(false)}
      onSubmit={() => canAllow && void answer(true)}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => void answer(false)}>{t.access.deny}</Button>
          <Button type="submit" variant="primary" icon={Check} loading={busy} disabled={!canAllow} autoFocus>{t.access.allow}</Button>
        </>
      }
    >
      {request.reason && <div className="agent-reason">{request.reason}</div>}
      <div className="agent-group">
        <span className="agent-label">{t.access.hosts}</span>
        {request.hosts.map((h) => (
          <Checkbox key={key(h)} checked={hosts.has(key(h))} onChange={(on) => setHosts((s) => toggle(s, key(h), on))}>
            {hostName(h)}
          </Checkbox>
        ))}
      </div>
      <div className="agent-group">
        <span className="agent-label">{t.access.rights}</span>
        {request.scopes.map((sc) => (
          <Checkbox key={sc} checked={scopes.has(sc)} onChange={(on) => setScopes((s) => toggle(s, sc, on))}>
            {t.scopes[sc]}
          </Checkbox>
        ))}
      </div>
      <div className="agent-group">
        <span className="agent-label">{t.access.ttl}</span>
        <Segmented value={ttl} onChange={setTtl} options={TTLS.map((v) => ({ value: v, label: t.ttl[v] }))} />
      </div>
    </Modal>
  );
}

function CommandDialog({ request, onDone }: { request: Extract<AgentRequest, { type: "command" }>; onDone: () => void }) {
  const t = useT().agents;
  const { toast } = useStore();
  const [always, setAlways] = useState(false);
  const answer = async (allow: boolean) => {
    try {
      await api.agentsAnswer(request.id, allow, { always: allow && always });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    onDone();
  };
  return (
    <Modal
      title={t.command.title(request.agent)}
      width={520}
      onClose={() => void answer(false)}
      onSubmit={() => void answer(true)}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={() => void answer(false)}>{t.command.deny}</Button>
          <Button type="submit" variant="primary" autoFocus>{t.command.allow}</Button>
        </>
      }
    >
      <span className="agent-label">{t.command.on(hostName(request.host))}</span>
      <pre className="export-preview mono agent-command">{request.command}</pre>
      <Checkbox checked={always} onChange={setAlways}>{t.command.always}</Checkbox>
    </Modal>
  );
}

/** Settings → AI agents: who has access to what, and revoking it. */
export function AgentsSettings() {
  const t = useT().agents;
  const { toast } = useStore();
  const [tokens, setTokens] = useState<AgentToken[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    const load = () => void api.agentsList().then(setTokens).catch(() => setTokens([]));
    load();
    return api.onCoreEvent((e) => {
      if (e.name === "agents.changed" || e.name === "unlocked") load();
    });
  }, []);

  const revoke = async (tk: AgentToken) => {
    setBusy(tk.id);
    try {
      await api.agentsRevoke(tk.id);
      setTokens((list) => list?.filter((x) => x.id !== tk.id) ?? null);
      toast({ text: t.settings.revoked(tk.agent), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(null);
  };

  if (!tokens) return null;
  return (
    <section className="settings-group">
      <h3 className="section-title">{t.settings.title}</h3>
      {tokens.length === 0 && <div className="setting muted small">{t.settings.none}</div>}
      {tokens.map((tk) => {
        const when = tk.expired ? t.settings.expired : tk.expiresAt ? t.settings.expires(new Date(tk.expiresAt).toLocaleString(locale(), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })) : t.settings.untilRevoked;
        return (
          <div key={tk.id} className="setting agent-token">
            <Bot size={18} className="agent-icon" />
            <div className="agent-token-main">
              <span className="agent-token-name">{tk.agent}</span>
              <span className="muted small" title={tk.hosts.map(hostName).join(", ")}>
                {tk.hosts.length === 1 ? hostName(tk.hosts[0]!) : t.settings.hosts(tk.hosts.length)} · {tk.scopes.map((s: AgentScope) => t.scopesShort[s]).join(", ")}
              </span>
              <span className={`small ${tk.expired ? "err-text" : "muted"}`}>
                {when}
                {tk.lastUsed ? ` · ${t.settings.used(formatAgo(tk.lastUsed))}` : ""}
              </span>
            </div>
            <Button size="sm" loading={busy === tk.id} onClick={() => void revoke(tk)}>{t.settings.revoke}</Button>
          </div>
        );
      })}
    </section>
  );
}

/** What an agent ran in its session, under the terminal: the last command, the rest on a click. */
export function AgentRuns({ sessionId, agent }: { sessionId: string; agent: string }) {
  const t = useT().agents;
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [open, setOpen] = useState(false);
  const [shown, setShown] = useState<number | null>(null);

  useEffect(() => {
    const load = () => void api.sessionRuns(sessionId).then(setRuns).catch(() => {});
    load();
    return api.onCoreEvent((e) => {
      if (e.name === "session.runs" && (e.data as { sessionId: string }).sessionId === sessionId) load();
    });
  }, [sessionId]);

  const last = runs[runs.length - 1];
  if (!last) return null;
  return (
    <div className={`agent-runs ${open ? "open" : ""}`}>
      {open && (
        <div className="agent-runs-list">
          {[...runs].reverse().map((r) => (
            <div key={r.id} className="agent-run">
              <button className="agent-run-head" onClick={() => setShown((s) => (s === r.id ? null : r.id))} aria-expanded={shown === r.id}>
                <RunState run={r} />
                <span className="agent-run-cmd mono">{r.command}</span>
                <span className="muted small">{formatAgo(r.startedAt)}</span>
              </button>
              {shown === r.id && <pre className="agent-run-output mono">{r.output || t.runs.noOutput}</pre>}
            </div>
          ))}
        </div>
      )}
      <button className="agent-runs-bar" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Bot size={14} className="agent-icon" />
        <span className="agent-runs-who">{agent}</span>
        <RunState run={last} />
        <span className="agent-run-cmd mono">{last.command}</span>
        {open ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
      </button>
    </div>
  );
}

function RunState({ run }: { run: AgentRun }) {
  const t = useT().agents.runs;
  if (!run.finishedAt) return <Loader2 size={13} className="spin muted" aria-label={t.running} />;
  if (run.timedOut) return <span className="run-code err">{t.timedOut}</span>;
  if (run.exitStatus === 0) return <Check size={13} className="ok-text" aria-label="0" />;
  return <span className="run-code err">{run.exitStatus ?? "?"}</span>;
}
