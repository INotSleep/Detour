import { useCallback, useEffect, useState } from "react";
import { ArrowRight, Ellipsis, History, Loader2, RotateCcw, Undo2 } from "lucide-react";
import { api, errorText, type DeletedRecord, type Rollback, type RollbackItem, type TeamChange, type TeamLogEntry } from "../lib/api";
import { formatAgo, useStore } from "../lib/store";
import type { Vault } from "../lib/types";
import { useT, type Dict } from "../i18n";
import { show } from "./ConflictDialog";
import { Button, IconButton, Menu, Modal } from "./ui";

/** Goes back and offers to undo it. */
function useRollback(vault: Vault) {
  const t = useT();
  const { toast, applyTeamView } = useStore();
  return useCallback(
    async (to: Rollback, onDone?: () => void) => {
      const go = async (to: Rollback, undoable: boolean) => {
        const r = await api.teamRollback(vault.id, to);
        applyTeamView(r.view);
        onDone?.();
        const entry = r.done.entry;
        if (!undoable) return toast({ text: t.teams.history.undone, tone: "success" });
        if (r.done.applied === 0) return toast({ text: t.teams.history.nothing });
        toast({
          text: t.teams.history.done(r.done.applied),
          tone: "success",
          action: entry ? { label: t.common.undo, run: () => void go({ kind: "undo", entry }, false).catch((e) => toast({ text: errorText(e), tone: "error" })) } : undefined,
        });
      };
      await go(to, true);
    },
    [vault.id, toast, applyTeamView, t],
  );
}

/** One change of a log entry, in words. */
function changeText(t: Dict, c: TeamChange): string {
  const a = t.teams.activity;
  const what = c.coll ? t.teams.coll[c.coll] : "";
  switch (c.kind) {
    case "created": return a.added(what, c.name);
    case "changed": return a.changed(what, c.name);
    case "deleted": return a.deleted(what, c.name);
    case "restored": return t.teams.history.restored(what, c.name);
    case "access": return a.accessChanged(c.member ?? "", c.level ? t.teams.level[c.level].toLowerCase() : "", c.name);
    case "teamCreated": return a.created;
    case "teamRenamed": return a.renamed(c.name);
    case "memberAdded": return a.memberAdded(c.name);
    case "memberChanged": return a.memberChanged(c.name);
    case "memberRemoved": return a.memberRemoved(c.name);
  }
}

/** Fields that are bookkeeping, not something a person set. */
const HIDDEN = new Set(["order", "forwardAgent"]);
const isEmpty = (v: unknown) => v === null || v === undefined || v === "" || v === false || (Array.isArray(v) && v.length === 0);

/** What a change set: "Address: 10.0.0.2 · Password: changed". */
function ChangeFields({ c }: { c: TeamChange }) {
  const t = useT();
  const label: Record<string, string> = t.teams.conflicts.field;
  // A new record: only what it was given, not its empty fields, nor the name already in the line above.
  const shown = Object.entries(c.fields).filter(([k, v]) => !HIDDEN.has(k) && !(c.kind === "created" && (k === "label" || k === "name" || isEmpty(v))));
  const parts = [...shown.map(([k, v]) => `${label[k] ?? k}: ${show(v)}`), ...c.secrets.map((k) => `${label[k] ?? k}: ${t.teams.history.secretSet}`)];
  if (parts.length === 0) return null;
  return <div className="history-fields mono">{parts.join(" · ")}</div>;
}

function whoAndWhere(t: Dict, e: TeamLogEntry): string {
  const device = e.thisDevice ? t.teams.history.thisDevice : t.teams.history.device(e.device.slice(0, 8));
  return `${e.isYou ? t.common.you : e.authorName} · ${device} · ${formatAgo(e.at)}`;
}

const PAGE = 40;

/** The team's History tab: deleted records to bring back, and every change with ways to go back. */
export function TeamHistory({ vault }: { vault: Vault }) {
  const t = useT();
  const { toast } = useStore();
  const rollback = useRollback(vault);
  const [log, setLog] = useState<TeamLogEntry[] | null>(null);
  const [deleted, setDeleted] = useState<DeletedRecord[]>([]);
  const [shown, setShown] = useState(PAGE);
  const [plan, setPlan] = useState<{ to: Rollback; title: string } | null>(null);
  const [restoring, setRestoring] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [l, d] = await Promise.all([api.teamLog(vault.id), api.teamDeleted(vault.id)]);
      setLog(l.reverse());
      setDeleted(d);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
      setLog((l) => l ?? []);
    }
  }, [vault.id, toast]);

  // Again whenever the team's entries change, here or on another device.
  useEffect(() => void load(), [load, vault.stateFingerprint]);

  const restore = async (d: DeletedRecord) => {
    setRestoring(d.record);
    try {
      await rollback({ kind: "restore", record: d.record }, load);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setRestoring(null);
  };

  if (!log) return <div className="history-loading"><Loader2 size={16} className="spin muted" /></div>;

  const h = t.teams.history;
  return (
    <div className="history">
      {deleted.length > 0 && (
        <section>
          <h3 className="section-title">
            {h.deleted} <span className="count">{deleted.length}</span>
          </h3>
          <div className="feed">
            {deleted.map((d) => (
              <div className="feed-item history-row" key={d.record}>
                <div className="grow">
                  <div className="strong-ish">{t.teams.conflicts.record(t.teams.coll[d.coll], d.name)}</div>
                  <div className="muted small">{d.isYou ? t.common.you : d.authorName} · {formatAgo(d.at)}</div>
                </div>
                <Button size="sm" icon={Undo2} loading={restoring === d.record} onClick={() => restore(d)}>{h.restore}</Button>
              </div>
            ))}
          </div>
        </section>
      )}

      <section>
        {log.length === 0 ? (
          <div className="muted small">{h.empty}</div>
        ) : (
          <div className="feed">
            {log.slice(0, shown).map((e) => {
              const records = e.changes.some((c) => c.record && c.kind !== "access");
              return (
                <div className="feed-item history-row" key={e.entry}>
                  <div className="grow">
                    {e.changes.map((c, i) => (
                      <div key={i}>
                        <div className="strong-ish">{changeText(t, c)}</div>
                        {c.kind === "changed" && <ChangeFields c={c} />}
                      </div>
                    ))}
                    <div className="muted small">{whoAndWhere(t, e)}</div>
                  </div>
                  <Menu
                    trigger={(toggle, open) => <IconButton icon={Ellipsis} label={t.teams.more} className={open ? "pressed" : ""} onClick={toggle} />}
                    items={() => [
                      ...(records ? [{ label: h.undo, icon: Undo2, onSelect: () => setPlan({ to: { kind: "undo", entry: e.entry }, title: h.plan.undo }) }] : []),
                      { label: h.backToHere, icon: RotateCcw, onSelect: () => setPlan({ to: { kind: "at", at: e.at }, title: h.plan.at }) },
                      { label: h.undoDevice, icon: History, onSelect: () => setPlan({ to: { kind: "without", device: e.device, author: null, after: e.at - 1 }, title: h.plan.without }) },
                    ]}
                  />
                </div>
              );
            })}
          </div>
        )}
        {log.length > shown && (
          <Button variant="ghost" className="history-more" onClick={() => setShown((n) => n + PAGE)}>{h.showMore}</Button>
        )}
      </section>

      {plan && <RollbackDialog vault={vault} to={plan.to} title={plan.title} onClose={() => setPlan(null)} onDone={load} />}
    </div>
  );
}

/** What going back would change, and going back. */
function RollbackDialog({ vault, to, title, onClose, onDone }: { vault: Vault; to: Rollback; title: string; onClose: () => void; onDone?: () => void }) {
  const t = useT();
  const h = t.teams.history;
  const label: Record<string, string> = t.teams.conflicts.field;
  const { toast } = useStore();
  const rollback = useRollback(vault);
  const [items, setItems] = useState<RollbackItem[] | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .teamRollbackPlan(vault.id, to)
      .then((p) => setItems(p.items))
      .catch((e) => {
        toast({ text: errorText(e), tone: "error" });
        onClose();
      });
    // The plan for what was asked when the dialog opened.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const allowed = items?.filter((i) => i.allowed).length ?? 0;
  const apply = async () => {
    setBusy(true);
    try {
      await rollback(to, onDone);
      onClose();
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
      setBusy(false);
    }
  };

  return (
    <Modal
      title={title}
      width={560}
      onClose={onClose}
      onSubmit={allowed > 0 ? apply : undefined}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button variant="primary" type="submit" icon={RotateCcw} loading={busy} disabled={allowed === 0}>{h.apply}</Button>
        </>
      }
    >
      {!items ? (
        <div className="history-loading"><Loader2 size={16} className="spin muted" /></div>
      ) : items.length === 0 ? (
        <div className="muted">{h.nothing}</div>
      ) : (
        <div className="plan">
          {items.map((i) => (
            <div className={`plan-item ${i.allowed ? "" : "denied"}`} key={i.record}>
              <div className="plan-head">
                <span className="strong-ish grow">{t.teams.conflicts.record(t.teams.coll[i.coll], i.name)}</span>
                {i.action === "restore" && <span className="chip">{h.willRestore}</span>}
                {i.action === "remove" && <span className="chip warn">{h.willRemove}</span>}
              </div>
              {i.action !== "remove" && i.fields.length > 0 && (
                <div className="plan-fields">
                  {i.fields.map((f) => (
                    <div className="plan-field" key={f.field}>
                      <span className="diff-field">{label[f.field] ?? f.field}</span>
                      {f.secret ? (
                        <span className="muted">{h.secretSet}</span>
                      ) : (
                        <span className="plan-change mono">
                          <span className="muted strike">{show(f.from)}</span>
                          <ArrowRight size={12} className="muted" />
                          <span>{show(f.to)}</span>
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
              {!i.allowed && <div className="muted small">{h.noAccess}</div>}
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}

/** Every version of one record of a team, each one can come back. */
export function RecordHistoryDialog({ vault, record, name, onClose }: { vault: Vault; record: string; name: string; onClose: () => void }) {
  const t = useT();
  const h = t.teams.history;
  const { toast } = useStore();
  const [versions, setVersions] = useState<TeamLogEntry[] | null>(null);
  const [plan, setPlan] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .teamHistory(vault.id, record)
      .then((v) => setVersions(v.reverse()))
      .catch((e) => {
        toast({ text: errorText(e), tone: "error" });
        onClose();
      });
  }, [vault.id, record, toast, onClose]);
  useEffect(() => load(), [load]);

  return (
    <Modal title={h.historyOf(name)} width={560} onClose={onClose} footer={<Button variant="ghost" onClick={onClose}>{t.common.close}</Button>}>
      {!versions ? (
        <div className="history-loading"><Loader2 size={16} className="spin muted" /></div>
      ) : (
        <div className="feed">
          {versions.map((e, i) => (
            <div className="feed-item history-row" key={e.entry}>
              <div className="grow">
                {e.changes.map((c, j) => (
                  <div key={j}>
                    <div className="strong-ish">{changeText(t, c)}</div>
                    {(c.kind === "changed" || c.kind === "created") && <ChangeFields c={c} />}
                  </div>
                ))}
                <div className="muted small">{whoAndWhere(t, e)}</div>
              </div>
              {i === 0 ? <span className="chip">{h.current}</span> : <Button size="sm" icon={Undo2} onClick={() => setPlan(e.entry)}>{h.useVersion}</Button>}
            </div>
          ))}
        </div>
      )}
      {plan && <RollbackDialog vault={vault} to={{ kind: "record", record, entry: plan }} title={h.plan.record} onClose={() => setPlan(null)} onDone={load} />}
    </Modal>
  );
}
