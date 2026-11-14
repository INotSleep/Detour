import { useEffect, useState } from "react";
import { Bot, Check, ChevronDown, ChevronUp, Loader2, Plus } from "lucide-react";
import { AGENT_SCOPES, api, errorText, type AgentReach, type AgentRequest, type AgentRun, type McpClient, type AgentScope, type AgentToken, type AgentTtl } from "../lib/api";
import { copyText, formatAgo, useStore } from "../lib/store";
import { listGroups } from "../lib/groups";
import type { Workspace } from "../lib/types";
import { Button, Checkbox, Field, Modal, Segmented } from "./ui";
import { locale, useT } from "../i18n";

const TTLS: AgentTtl[] = ["2h", "1d", "7d", "never"];
const hostName = (h: { label: string; team: string | null }) => (h.team ? `${h.team} / ${h.label}` : h.label);

/** What an agent waits on: access for a token, or a command of a token that asks each time. */
export function AgentRequestDialog({ request, onDone }: { request: AgentRequest; onDone: () => void }) {
  return request.type === "access" ? <AccessDialog request={request} onDone={onDone} /> : <CommandDialog request={request} onDone={onDone} />;
}

function AccessDialog({ request, onDone }: { request: Extract<AgentRequest, { type: "access" }>; onDone: () => void }) {
  const t = useT().agents;
  const { ws, toast } = useStore();
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

  // Instead of the hosts asked for: everything in their vault, or in their group, so the next ones don't ask.
  const vaults = [...new Set(request.hosts.map((h) => h.vaultId))];
  const vault = vaults.length === 1 ? ws.vaults.find((v) => v.id === vaults[0]) : undefined;
  const groupsOf = [...new Set(request.hosts.map((h) => ws.hosts.find((x) => x.id === h.hostId && x.vaultId === h.vaultId)?.group ?? ""))];
  const group = vault && groupsOf.length === 1 && groupsOf[0] ? groupsOf[0] : null;
  const [wide, setWide] = useState<"asked" | "vault" | "group">("asked");
  const reach: AgentReach[] = vault && wide === "vault" ? [{ type: "vault", vaultId: vault.id }] : vault && group && wide === "group" ? [{ type: "group", vaultId: vault.id, name: group }] : [];

  const answer = async (allow: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      const picked = reach.length ? [] : request.hosts.filter((h) => hosts.has(key(h))).map((h) => ({ vaultId: h.vaultId, hostId: h.hostId }));
      const got = await api.agentsAnswer(request.id, allow, allow ? { ttl, scopes: [...scopes], hosts: picked, reach } : {});
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

  const canAllow = (reach.length > 0 || hosts.size > 0) && scopes.size > 0;
  return (
    <Modal
      title={t.access.title(request.agent)}
      width={500}
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
        {vault && (
          <Segmented
            value={wide}
            onChange={setWide}
            options={[
              { value: "asked" as const, label: t.widen.asked },
              ...(group ? [{ value: "group" as const, label: t.widen.group(group) }] : []),
              { value: "vault" as const, label: t.widen.vault(vault.name) },
            ]}
          />
        )}
        {wide === "asked" &&
          request.hosts.map((h) => (
            <Checkbox key={key(h)} checked={hosts.has(key(h))} onChange={(on) => setHosts((s) => toggle(s, key(h), on))}>
              {hostName(h)}
            </Checkbox>
          ))}
      </div>
      <div className="agent-group">
        <span className="agent-label">{t.access.rights}</span>
        <div className="agent-scopes">
          {AGENT_SCOPES.map((sc) => (
          <Checkbox key={sc} checked={scopes.has(sc)} onChange={(on) => setScopes((s) => toggle(s, sc, on))}>
            {t.scopes[sc]}
          </Checkbox>
          ))}
        </div>
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

/** What a token reaches, briefly: "All hosts", "Acme", "Acme: Prod", host names. */
function reachParts(ws: Workspace, tk: AgentToken, t: ReturnType<typeof useT>["agents"]): string[] {
  const vaultName = (id: string) => ws.vaults.find((v) => v.id === id)?.name ?? id;
  const parts = tk.reach.map((r) => {
    if (r.type === "all") return t.rule.all;
    if (r.type === "vault") return vaultName(r.vaultId);
    if (r.type === "group") return `${vaultName(r.vaultId)}: ${r.name}`;
    const g = ws.layouts[r.vaultId]?.groups.find((x) => x.id === r.id);
    return `${vaultName(r.vaultId)}: ${g?.name ?? "?"} (${t.rule.mine})`;
  });
  return [...parts, ...tk.named.map(hostName)];
}

/** Settings → AI agents: who has access to what, changing it, giving it ahead and revoking it. */
export function AgentsSettings() {
  const t = useT().agents;
  const { ws, toast } = useStore();
  const [tokens, setTokens] = useState<AgentToken[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [editing, setEditing] = useState<AgentToken | "new" | null>(null);

  const load = () => void api.agentsList().then(setTokens).catch(() => setTokens([]));
  useEffect(() => {
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
      <div className="section-head">
        <h3 className="section-title">{t.settings.title}</h3>
        <Button size="sm" icon={Plus} onClick={() => setEditing("new")}>{t.settings.add}</Button>
      </div>
      {tokens.length === 0 && <div className="setting muted small">{t.settings.none}</div>}
      {tokens.map((tk) => {
        const when = tk.expired ? t.settings.expired : tk.expiresAt ? t.settings.expires(new Date(tk.expiresAt).toLocaleString(locale(), { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })) : t.settings.untilRevoked;
        const parts = reachParts(ws, tk, t);
        const shown = parts.length > 3 ? [...parts.slice(0, 2), t.settings.more(parts.length - 2)] : parts;
        return (
          <div key={tk.id} className="setting agent-token">
            <Bot size={18} className="agent-icon" />
            <div className="agent-token-main">
              <span className="agent-token-name">{tk.agent}</span>
              <span className="muted small" title={parts.join(", ")}>
                {shown.join(", ")} · {tk.scopes.map((s: AgentScope) => t.scopesShort[s]).join(", ")}
              </span>
              <span className={`small ${tk.expired ? "err-text" : "muted"}`}>
                {when}
                {tk.lastUsed ? ` · ${t.settings.used(formatAgo(tk.lastUsed))}` : ""}
              </span>
            </div>
            <Button size="sm" variant="ghost" onClick={() => setEditing(tk)}>{t.settings.edit}</Button>
            <Button size="sm" loading={busy === tk.id} onClick={() => void revoke(tk)}>{t.settings.revoke}</Button>
          </div>
        );
      })}
      {editing && (
        <RuleDialog
          token={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            load();
          }}
        />
      )}
    </section>
  );
}

const OTHER = "other:";

/** Access made or changed by the person: an agent, the hosts it reaches, what it may do, for how long. */
function RuleDialog({ token, onClose, onSaved }: { token: AgentToken | null; onClose: () => void; onSaved: () => void }) {
  const tt = useT();
  const t = tt.agents;
  const { ws, toast } = useStore();
  const [clients, setClients] = useState<McpClient[] | null>(null);
  useEffect(() => void api.mcpClients().then((c) => setClients(c.filter((x) => x.installed))).catch(() => setClients([])), []);
  const [client, setClient] = useState<string | null>(null);
  const picked = client ?? clients?.[0]?.id ?? OTHER;
  const [name, setName] = useState("");

  // Picks by key: "all", "v|<vault>", "g|<vault>|<name>", "m|<vault>|<id>", "h|<vault>|<host>".
  const [picks, setPicks] = useState<Set<string>>(() => {
    const out = new Set<string>();
    for (const r of token?.reach ?? []) {
      out.add(r.type === "all" ? "all" : r.type === "vault" ? `v|${r.vaultId}` : r.type === "group" ? `g|${r.vaultId}|${r.name}` : `m|${r.vaultId}|${r.id}`);
    }
    for (const h of token?.named ?? []) out.add(`h|${h.vaultId}|${h.hostId}`);
    return out;
  });
  const [scopes, setScopes] = useState<Set<AgentScope>>(() => new Set(token?.scopes ?? ["exec", "hostkeys:new"]));
  const [ttl, setTtl] = useState<AgentTtl | null>(token ? null : "never");
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const pick = (k: string, on: boolean) =>
    setPicks((s) => {
      const next = new Set(s);
      if (on) next.add(k);
      else next.delete(k);
      return next;
    });

  const reach: AgentReach[] = [];
  const hosts: { vaultId: string; hostId: string }[] = [];
  for (const k of picks) {
    const [kind, v = "", ...rest] = k.split("|");
    const x = rest.join("|");
    if (kind === "all") reach.push({ type: "all" });
    else if (kind === "v") reach.push({ type: "vault", vaultId: v });
    else if (kind === "g") reach.push({ type: "group", vaultId: v, name: x });
    else if (kind === "m") reach.push({ type: "myGroup", vaultId: v, id: x });
    else if (kind === "h") hosts.push({ vaultId: v, hostId: x });
  }
  const agent = token?.agent ?? (picked === OTHER ? name.trim() : clients?.find((c) => c.id === picked)?.name ?? "");
  const error = !agent ? t.rule.nameRequired : !reach.length && !hosts.length ? t.rule.pickHosts : !scopes.size ? t.rule.pickRights : null;

  const save = async () => {
    setTouched(true);
    if (error || busy) return;
    setBusy(true);
    try {
      const got = await api.agentsSet({ id: token?.id, agent, reach, hosts, scopes: [...scopes], ttl: ttl ?? undefined });
      if (token) toast({ text: t.rule.saved, tone: "success" });
      else if (picked !== OTHER && got.token) {
        // Not in the client's config, the access would only wait there unused.
        await api.mcpClientToken(picked, got.token).catch(async (e) => {
          await api.agentsRevoke(got.tokenId).catch(() => {});
          throw e;
        });
        toast({ text: t.rule.given(agent), tone: "success" });
      } else if (got.token) {
        const secret = got.token;
        toast({ text: t.rule.givenCopy(agent), tone: "success", action: { label: t.access.copyToken, run: () => void copyText(secret).then((ok) => ok && toast({ text: t.access.tokenCopied, tone: "success" })) } });
      }
      onSaved();
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(false);
  };

  const all = picks.has("all");
  const vaults = [...ws.vaults].sort((a, b) => (a.kind === "personal" ? -1 : b.kind === "personal" ? 1 : 0));
  if (!clients) return null;
  return (
    <Modal
      title={token ? t.rule.title(token.agent) : t.rule.titleNew}
      width={560}
      onClose={onClose}
      onSubmit={() => void save()}
      footer={
        <>
          {touched && error && <span className="err-text small modal-foot-left">{error}</span>}
          <Button type="button" variant="ghost" onClick={onClose}>{tt.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy}>{token ? t.rule.save : t.rule.give}</Button>
        </>
      }
    >
      {!token && (
        <div className="agent-group">
          <span className="agent-label">{t.rule.agent}</span>
          <Segmented value={picked} onChange={setClient} options={[...clients.map((c) => ({ value: c.id, label: c.name })), { value: OTHER, label: t.rule.other }]} />
          {picked === OTHER && (
            <Field label={t.rule.name}>
              <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Codex" autoFocus />
            </Field>
          )}
        </div>
      )}
      <div className="agent-group">
        <span className="agent-label">{t.rule.hosts}</span>
        <div className="agent-reach">
          <Checkbox checked={all} onChange={(on) => pick("all", on)}>{t.rule.all}</Checkbox>
          {!all &&
            vaults.map((v) => {
              const whole = picks.has(`v|${v.id}`);
              const vHosts = ws.hosts.filter((h) => h.vaultId === v.id);
              const shared = listGroups(vHosts, ws.groups.filter((g) => g.vaultId === v.id), false).filter((g) => g.name);
              const mine = v.kind === "team" ? ws.layouts[v.id]?.groups ?? [] : [];
              if (!vHosts.length) return null;
              return (
                <div key={v.id} className="agent-vault">
                  <Checkbox checked={whole} onChange={(on) => pick(`v|${v.id}`, on)}>{t.rule.whole(v.name)}</Checkbox>
                  {!whole && (shared.length > 0 || mine.length > 0) && (
                    <div className="agent-picks">
                      <span className="agent-sub">{t.rule.groups}</span>
                      {shared.map((g) => (
                        <Checkbox key={g.key} checked={picks.has(`g|${v.id}|${g.name}`)} onChange={(on) => pick(`g|${v.id}|${g.name}`, on)}>{g.name}</Checkbox>
                      ))}
                      {mine.map((g) => (
                        <Checkbox key={g.id} checked={picks.has(`m|${v.id}|${g.id}`)} onChange={(on) => pick(`m|${v.id}|${g.id}`, on)}>
                          {g.name} <span className="muted">({t.rule.mine})</span>
                        </Checkbox>
                      ))}
                    </div>
                  )}
                  {!whole && (
                    <div className="agent-picks">
                      <span className="agent-sub">{t.rule.single}</span>
                      {vHosts.map((h) => (
                        <Checkbox key={h.id} checked={picks.has(`h|${v.id}|${h.id}`)} onChange={(on) => pick(`h|${v.id}|${h.id}`, on)}>{h.label}</Checkbox>
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
        </div>
      </div>
      <div className="agent-group">
        <span className="agent-label">{t.access.rights}</span>
        <div className="agent-scopes">
          {AGENT_SCOPES.map((sc) => (
          <Checkbox
            key={sc}
            checked={scopes.has(sc)}
            onChange={(on) =>
              setScopes((s) => {
                const next = new Set(s);
                if (on) next.add(sc);
                else next.delete(sc);
                return next;
              })
            }
          >
            {t.scopes[sc]}
          </Checkbox>
          ))}
        </div>
      </div>
      <div className="agent-group">
        <span className="agent-label">{t.access.ttl}</span>
        <Segmented value={ttl ?? ("" as AgentTtl)} onChange={setTtl} options={TTLS.map((v) => ({ value: v, label: t.ttl[v] }))} />
      </div>
    </Modal>
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

/** Settings: adding Detour's MCP server to the AI clients on this computer. */
export function McpClients() {
  const t = useT().agents.clients;
  const { toast } = useStore();
  const [clients, setClients] = useState<McpClient[]>([]);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => void api.mcpClients().then(setClients).catch(() => {}), []);

  const set = async (c: McpClient, on: boolean) => {
    setBusy(c.id);
    try {
      await api.mcpClientSet(c.id, on);
      setClients(await api.mcpClients());
      toast({ text: on ? t.added(c.name) : t.removed(c.name), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(null);
  };

  if (!clients.length) return null;
  return (
    <section className="settings-group">
      <h3 className="section-title">{t.title}</h3>
      {clients.map((c) => (
        <div key={c.id} className="setting">
          <span>{c.name}</span>
          <div className="mcp-client-actions">
            {c.added && c.current && <Check size={15} className="ok-text" aria-label={t.add} />}
            {c.added && <Button size="sm" variant="ghost" disabled={busy === c.id} onClick={() => void set(c, false)}>{t.remove}</Button>}
            {!(c.added && c.current) && (
              <Button size="sm" loading={busy === c.id} onClick={() => void set(c, true)}>
                {c.added ? t.update : t.add}
              </Button>
            )}
          </div>
        </div>
      ))}
    </section>
  );
}
