import { useState } from "react";
import { api, errorText } from "../lib/api";
import { formatAgo, useStore } from "../lib/store";
import type { Coll, Conflict, Vault, Workspace } from "../lib/types";
import { Button, Modal } from "./ui";
import { tr, useT } from "../i18n";

function recordName(ws: Workspace, coll: Coll, id: string) {
  const list = coll === "host" ? ws.hosts : coll === "key" ? ws.keys : coll === "snippet" ? ws.snippets : ws.tunnels;
  const r = list.find((x) => x.id === id) as { label?: string; name?: string } | undefined;
  return r?.label ?? r?.name ?? id;
}

function show(v: unknown): string {
  const empty = tr().teams.conflicts.empty;
  if (Array.isArray(v)) return v.join(", ") || empty;
  if (v && typeof v === "object") return "type" in v ? String((v as { type: string }).type) : JSON.stringify(v);
  if (v === null || v === undefined || v === "") return empty;
  return String(v);
}

type Group = { coll: Coll; record: string; fields: Extract<Conflict, { kind: "field" }>[]; deleted?: Extract<Conflict, { kind: "deleted" }> };

export function ConflictDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
  const t = useT();
  const fieldLabel: Record<string, string> = t.teams.conflicts.field;
  const { ws, update, toast, applyTeamView } = useStore();
  const you = ws.members.find((m) => m.vaultId === vault.id && m.isYou)?.id;

  const groups: Group[] = [];
  for (const c of vault.conflicts) {
    let g = groups.find((x) => x.coll === c.coll && x.record === c.record);
    if (!g) groups.push((g = { coll: c.coll, record: c.record, fields: [] }));
    if (c.kind === "field") g.fields.push(c);
    else g.deleted = c;
  }

  // Default choice for each field: the latest value.
  const [picks, setPicks] = useState<Record<string, number>>(() =>
    Object.fromEntries(vault.conflicts.filter((c) => c.kind === "field").map((c) => [`${c.coll}/${c.record}/${c.field}`, c.values.length - 1])),
  );
  const [busy, setBusy] = useState<string | null>(null);

  const finish = () => {
    toast({ text: t.teams.conflicts.resolved, tone: "success" });
  };

  const applyFields = async (g: Group) => {
    setBusy(g.record);
    try {
      if (api.real) {
        for (const f of g.fields) applyTeamView(await api.teamResolveField(vault.id, g.coll, g.record, f.field, picks[`${g.coll}/${g.record}/${f.field}`] ?? 0));
      } else {
        update((w) => ({
          ...w,
          hosts: w.hosts.map((h) => (h.id === g.record ? { ...h, ...Object.fromEntries(g.fields.map((f) => [f.field, f.values[picks[`${g.coll}/${g.record}/${f.field}`] ?? 0]!.value])) } : h)),
          vaults: w.vaults.map((v) => (v.id === vault.id ? dropConflicts(v, g) : v)),
        }));
      }
      finish();
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(null);
  };

  const resolveDeleted = async (g: Group, keep: boolean) => {
    setBusy(g.record);
    try {
      if (api.real) applyTeamView(await api.teamResolveDeleted(vault.id, g.coll, g.record, keep));
      else
        update((w) => ({
          ...w,
          hosts: keep ? w.hosts : w.hosts.filter((h) => h.id !== g.record),
          vaults: w.vaults.map((v) => (v.id === vault.id ? dropConflicts(v, g) : v)),
        }));
      finish();
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(null);
  };

  if (groups.length === 0) {
    onClose();
    return null;
  }

  return (
    <Modal title={t.teams.conflicts.title(vault.name)} width={620} onClose={onClose}>
      {groups.map((g) => (
        <section className="conflict-item" key={`${g.coll}/${g.record}`}>
          <div className="conflict-head">
            <div>
              <div className="strong">{t.teams.conflicts.record(t.teams.coll[g.coll], recordName(ws, g.coll, g.record))}</div>
              {g.deleted && (
                <div className="muted small">
                  {g.deleted.by === you ? t.teams.conflicts.deletedByYou : t.teams.conflicts.deletedBy(g.deleted.byName)} · {formatAgo(g.deleted.at)}
                </div>
              )}
            </div>
          </div>

          {g.fields.length > 0 && (
            <div className="choice-table">
              {g.fields.map((f) => {
                const key = `${g.coll}/${g.record}/${f.field}`;
                return (
                  <div className="choice-row" key={key}>
                    <span className="diff-field">{fieldLabel[f.field] ?? f.field}</span>
                    <div className="choices" role="radiogroup">
                      {f.values.map((v, i) => (
                        <button key={i} type="button" role="radio" aria-checked={picks[key] === i} className={`choice ${picks[key] === i ? "on" : ""}`} onClick={() => setPicks((p) => ({ ...p, [key]: i }))}>
                          <span className="choice-value mono">{show(v.value)}</span>
                          <span className="choice-who">{v.author === you ? t.common.you : v.authorName} · {formatAgo(v.at)}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          <div className="conflict-actions">
            {g.deleted ? (
              <>
                <Button variant="ghost" className="danger-text" loading={busy === g.record} onClick={() => resolveDeleted(g, false)}>{t.common.delete}</Button>
                <Button variant="primary" loading={busy === g.record} onClick={() => resolveDeleted(g, true)}>{t.teams.conflicts.keep}</Button>
              </>
            ) : (
              <Button variant="primary" loading={busy === g.record} onClick={() => applyFields(g)}>{t.teams.conflicts.apply}</Button>
            )}
          </div>
        </section>
      ))}
    </Modal>
  );
}

function dropConflicts(v: Vault, g: Group): Vault {
  const conflicts = v.conflicts.filter((c) => !(c.coll === g.coll && c.record === g.record));
  return { ...v, conflicts, syncStatus: conflicts.length ? "conflict" : "synced" };
}
