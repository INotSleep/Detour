import { Fragment, useEffect, useState } from "react";
import { ChevronRight, Copy, Ellipsis, Fingerprint, IdCard, LogIn, LogOut, Plus, RefreshCw, ShieldAlert, ShieldQuestion, TriangleAlert, UserMinus, UserPlus, Users, UserX } from "lucide-react";
import { api, errorText, isCoreError, type CoreError, type PendingTeam } from "../lib/api";
import { copyText, formatAgo, sleep, useStore } from "../lib/store";
import { atLeast, canManage, removeVault } from "../lib/teams";
import type { ActivityItem, Coll, Level, Member, Problem, Role, Vault } from "../lib/types";
import { tr, useT, type Dict } from "../i18n";
import { Avatar, SyncIcon, VaultAvatar, syncTarget } from "./common";
import { Button, ConfirmDialog, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented } from "./ui";
import { AddMemberDialog, CreateTeamDialog, JoinTeamDialog, MyIdDialog, useMyId } from "./TeamDialogs";

export function TeamsView() {
  const { vault } = useStore();
  return vault.kind === "team" ? <TeamPage vault={vault} /> : <TeamsOverview />;
}

/** Levels a record of this kind can have. */
const levelsFor = (coll: Coll): Level[] => (coll === "host" || coll === "key" ? ["none", "view", "secret", "edit"] : ["none", "view", "edit"]);

function activityText(t: Dict, a: ActivityItem): string {
  const x = t.teams.activity;
  switch (a.kind) {
    case "created": return x.created;
    case "renamed": return x.renamed(a.name);
    case "memberAdded": return x.memberAdded(a.member);
    case "memberChanged": return x.memberChanged(a.member);
    case "memberRemoved": return x.memberRemoved(a.member);
    case "accessChanged": return x.accessChanged(a.member, t.teams.level[a.level].toLowerCase(), a.name);
    case "secretChanged": return a.coll === "key" ? x.keyChanged(a.name) : x.passwordChanged(a.name);
    default: return x[a.kind](t.teams.coll[a.coll], a.name);
  }
}

/** Why a team's repository could not be reached, in words. */
export function remoteErrorText(e: CoreError): string {
  const t = tr().teams.remote;
  switch (e.kind) {
    case "hostKeyChanged": return t.hostKeyChanged;
    case "unreachable": case "connect": case "io": return t.unreachable;
    case "authFailed": return t.authFailed;
    case "needSecret": return t.needSecret;
    default: return errorText(e);
  }
}

export function problemText(p: Problem): string {
  const t = tr().teams.problem;
  const who = p.kind === "unknownFile" ? "" : p.authorName || t.device(p.device.slice(0, 8));
  switch (p.kind) {
    case "rejected": return t.rejected(who);
    case "missing": return t.missing(who);
    case "forked": return t.forked(who);
    case "rolledBack": return t.rolledBack(who);
    case "unknownFile": return t.unknownFile(p.path);
  }
}

/** Which members may I change? Owners manage everyone; admins manage members. */
function canEditMember(me: Role | undefined, m: Member) {
  if (m.isYou || m.recovery || !canManage(me)) return false;
  return me === "owner" || !canManage(m.role);
}

const memberRank = (m: Member) => (m.recovery ? 9 : m.isYou ? 0 : { owner: 1, admin: 2, member: 3 }[m.role]);
const byRank = (a: Member, b: Member) => memberRank(a) - memberRank(b) || a.name.localeCompare(b.name);

function TeamPage({ vault }: { vault: Vault }) {
  const t = useT();
  const { ws, update, toast, applyTeamView, resolveConflicts, setVault, sync: personalSync } = useStore();
  // With sync on, leaving takes the team off all of this person's devices.
  const everywhere = !!personalSync?.enabled;
  const me = useMyId();
  const [tab, setTab] = useState<"members" | "access">("members");
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<Member | null>(null);
  const [showFingerprint, setShowFingerprint] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [showProblems, setShowProblems] = useState(false);
  const [tokenDialog, setTokenDialog] = useState(false);
  const tokenRejected = vault.remoteError?.kind === "git" && vault.remoteError.message.includes("token rejected");

  const members = ws.members.filter((m) => m.vaultId === vault.id).sort(byRank);
  const activity = ws.activity.filter((a) => a.vaultId === vault.id).sort((a, b) => b.at - a.at);
  const manage = !!vault.joined && canManage(vault.myRole);
  const offline = vault.syncStatus === "offline";
  const target = syncTarget(vault.sync);
  const newServerKey = vault.remoteError?.kind === "hostKeyUnknown" ? vault.remoteError.fingerprint : null;
  const problems = vault.problems ?? [];

  const sync = async (trustFingerprint?: string) => {
    setSyncing(true);
    try {
      if (api.real) {
        const view = await api.teamSync(vault.id, trustFingerprint);
        applyTeamView(view);
        const failed = view.remote?.error;
        if (failed) {
          if (failed.kind !== "hostKeyUnknown") toast({ text: remoteErrorText(failed), tone: "error" });
        } else toast(view.conflicts.length ? { text: t.teams.page.conflictFound, tone: "error", action: { label: t.teams.page.resolve, run: () => resolveConflicts(vault.id) } } : { text: t.common.syncStatus.synced, tone: "success" });
      } else {
        await sleep(900);
        update((w) => ({ ...w, vaults: w.vaults.map((v) => (v.id === vault.id ? { ...v, lastSync: Date.now() } : v)) }));
        toast({ text: t.common.syncStatus.synced, tone: "success" });
      }
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setSyncing(false);
  };

  const replaceKey = async (fingerprint: string) => {
    setSyncing(true);
    try {
      const view = await api.teamReplaceServerKey(vault.id, fingerprint);
      applyTeamView(view);
      toast(view.remote?.error ? { text: remoteErrorText(view.remote.error), tone: "error" } : { text: t.teams.page.serverKeyReplaced, tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setSyncing(false);
  };

  const changeRole = async (m: Member, role: Role) => {
    try {
      if (api.real) applyTeamView(await api.teamMemberUpdate(vault.id, m.id, role));
      else update((w) => ({ ...w, members: w.members.map((x) => (x.id === m.id && x.vaultId === vault.id ? { ...x, role } : x)) }));
      toast({ text: t.teams.members.roleChanged(m.name, t.common.role[role].toLowerCase()), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const removeMember = async (m: Member) => {
    try {
      if (api.real) applyTeamView(await api.teamMemberRemove(vault.id, m.id));
      else update((w) => ({ ...w, members: w.members.filter((x) => !(x.id === m.id && x.vaultId === vault.id)), activity: [...w.activity, { vaultId: vault.id, authorName: t.common.you, isYou: true, at: Date.now(), kind: "memberRemoved", member: m.name }] }));
      toast({ text: t.teams.activity.memberRemoved(m.name), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const leave = async () => {
    try {
      if (api.real) await api.teamLeave(vault.id);
      setVault("personal");
      update((w) => removeVault(w, vault.id));
      toast({ text: everywhere ? t.teams.leave.doneEverywhere(vault.name) : t.teams.leave.doneHere(vault.name), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const copyMyId = async () => me && (await copyText(me.detourId)) && toast({ text: t.teams.page.idCopied, tone: "success" });

  return (
    <div className={`page ${tab === "access" ? "wide" : ""}`}>
      <PageHeader title={vault.name}>
        <Button icon={RefreshCw} loading={syncing} onClick={() => sync()}>{syncing ? t.teams.page.syncing : t.teams.page.sync}</Button>
        {manage && <Button variant="primary" icon={UserPlus} onClick={() => setAdding(true)}>{t.teams.page.addMember}</Button>}
        <Menu
          trigger={(toggle, open) => <IconButton icon={Ellipsis} label={t.teams.more} className={open ? "pressed" : ""} onClick={toggle} />}
          items={[
            ...(vault.stateFingerprint ? [{ label: t.teams.page.stateFingerprint, icon: Fingerprint, onSelect: () => setShowFingerprint(true) }] : []),
            { label: everywhere ? t.teams.leave.everywhere : t.teams.leave.here, icon: LogOut, danger: true, onSelect: () => setLeaving(true) },
          ]}
        />
      </PageHeader>

      <div className="sync-line">
        <SyncIcon sync={vault.sync} />
        <span className="mono ellipsis sync-path" title={target}>{target}</span>
        <span className="sep" />
        {vault.syncStatus === "conflict" ? (
          <button className="chip warn chip-btn" onClick={() => resolveConflicts(vault.id)}>
            <TriangleAlert size={12} /> {t.teams.page.resolveConflict}
          </button>
        ) : vault.remoteError ? (
          <span className="vault-sync conflict">
            <span className="dot" />
            {t.teams.page.noConnection}
          </span>
        ) : (
          <span className={`vault-sync ${vault.syncStatus}`}>
            <span className="dot" />
            {t.common.syncStatus[vault.syncStatus]}
          </span>
        )}
        {!offline && !vault.remoteError && vault.lastSync && <span className="muted">· {formatAgo(vault.lastSync)}</span>}
        {vault.pending && <span className="chip">{t.teams.page.notSent}</span>}
      </div>

      {newServerKey && (
        <div className="banner info">
          <ShieldQuestion size={16} />
          <span className="grow">{t.teams.page.newServerKey} <span className="mono">{newServerKey}</span></span>
          <Button size="sm" variant="primary" loading={syncing} onClick={() => sync(newServerKey)}>{t.teams.page.trust}</Button>
        </div>
      )}

      {vault.remoteError?.kind === "hostKeyChanged" && (
        <div className="banner warn">
          <ShieldAlert size={16} />
          <span className="grow">
            {t.teams.page.serverKeyChanged}
            <span className="banner-sub mono">{t.teams.page.keyWas} {vault.remoteError.expected}</span>
            <span className="banner-sub mono">{t.teams.page.keyNow} {vault.remoteError.actual}</span>
          </span>
          <Button size="sm" variant="ghost" className="danger-text" loading={syncing} onClick={() => replaceKey(vault.remoteError?.kind === "hostKeyChanged" ? vault.remoteError.actual : "")}>{t.teams.page.replaceKey}</Button>
        </div>
      )}

      {vault.remoteError && !newServerKey && vault.remoteError.kind !== "hostKeyChanged" && (
        <div className="banner warn">
          <TriangleAlert size={16} />
          <span className="grow">{remoteErrorText(vault.remoteError)}</span>
          {tokenRejected ? (
            <Button size="sm" variant="primary" onClick={() => setTokenDialog(true)}>{t.teams.page.newToken}</Button>
          ) : (
            <Button size="sm" loading={syncing} onClick={() => sync()}>{t.common.retry}</Button>
          )}
        </div>
      )}

      {offline && (
        <div className="banner warn">
          <TriangleAlert size={16} />
          <span className="grow">{t.teams.page.folderUnavailable}</span>
          <Button size="sm" loading={syncing} onClick={() => sync()}>{t.common.retry}</Button>
        </div>
      )}

      {!offline && vault.removed && (
        <div className="banner warn">
          <UserX size={16} />
          <span className="grow">{t.teams.page.youWereRemoved}</span>
          <Button size="sm" onClick={leave}>{everywhere ? t.teams.leave.everywhere : t.teams.leave.hereShort}</Button>
        </div>
      )}

      {!offline && vault.joined === false && !vault.removed && (
        <div className="banner info">
          <IdCard size={16} />
          <span className="grow">{t.teams.page.notAddedYet}</span>
          <Button size="sm" icon={Copy} onClick={copyMyId}>{t.teams.page.copyId}</Button>
          <Button size="sm" variant="primary" loading={syncing} onClick={() => sync()}>{t.teams.page.check}</Button>
        </div>
      )}

      {problems.length > 0 && (
        <div className="banner warn problems">
          <button className="problems-head" onClick={() => setShowProblems((s) => !s)} aria-expanded={showProblems}>
            <ShieldAlert size={16} />
            <span className="grow">{t.teams.problem.banner(problems.length)}</span>
            <ChevronRight size={14} className={`chev ${showProblems ? "open" : ""}`} />
          </button>
          {showProblems && (
            <ul className="problems-list">
              {problems.map((p, i) => <li key={i}>{problemText(p)}</li>)}
            </ul>
          )}
        </div>
      )}

      {vault.joined && !offline && (
        <>
          <div className="team-tabs">
            <Segmented value={tab} onChange={setTab} options={[{ value: "members", label: t.teams.page.tabMembers }, { value: "access", label: t.teams.page.tabAccess }]} />
          </div>

          {tab === "access" ? (
            <AccessMatrix vault={vault} />
          ) : (
            <div className="team-grid">
              <section>
                <div className="member-head">
                  <h3 className="section-title">
                    {t.teams.members.title} <span className="count">{members.length}</span>
                  </h3>
                  <span className="section-title">{t.teams.members.role}</span>
                  <span />
                </div>
                <div className="rows">
                  {members.map((m) => {
                    const editable = canEditMember(vault.myRole, m);
                    return (
                      <div className="member-row" key={m.id}>
                        <Avatar member={m} size={32} />
                        <div className="member-main">
                          <span className="member-name">
                            {m.recovery ? t.teams.members.recoveryKey : m.name}
                            {m.isYou && <span className="chip">{t.common.you.toLowerCase()}</span>}
                          </span>
                          <span className="member-sub mono">{m.fingerprint}</span>
                        </div>
                        {editable ? (
                          <select className="input input-sm role-select" value={m.role} onChange={(e) => changeRole(m, e.target.value as Role)}>
                            {vault.myRole === "owner" && <option value="owner">{t.common.role.owner}</option>}
                            {vault.myRole === "owner" && <option value="admin">{t.common.role.admin}</option>}
                            <option value="member">{t.common.role.member}</option>
                          </select>
                        ) : (
                          <span className="member-role">{t.common.role[m.role]}</span>
                        )}
                        <div className="member-actions">
                          {editable && (
                            <Menu
                              trigger={(toggle, open) => <IconButton icon={Ellipsis} label={t.teams.more} className={open ? "pressed" : ""} onClick={toggle} />}
                              items={[{ label: t.teams.members.removeFromTeam, icon: UserMinus, danger: true, onSelect: () => setRemoving(m) }]}
                            />
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </section>

              <section>
                <h3 className="section-title">{t.teams.activity.title}</h3>
                {activity.length === 0 ? (
                  <div className="muted small">{t.teams.activity.empty}</div>
                ) : (
                  <div className="feed">
                    {activity.slice(0, 30).map((a, i) => (
                      <div className="feed-item" key={i}>
                        <div>
                          <div className="strong-ish">{activityText(t, a)}</div>
                          <div className="muted small">{a.isYou ? t.common.you : a.authorName} · {formatAgo(a.at)}</div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </section>
            </div>
          )}
        </>
      )}

      {adding && <AddMemberDialog vault={vault} onClose={() => setAdding(false)} />}
      {tokenDialog && <TokenDialog vault={vault} onClose={() => setTokenDialog(false)} />}
      {removing && (
        <ConfirmDialog
          title={t.teams.members.removeTitle(removing.name)}
          body={t.teams.members.removeBody}
          confirm={t.teams.members.removeConfirm}
          onConfirm={() => removeMember(removing)}
          onClose={() => setRemoving(null)}
        />
      )}
      {showFingerprint && vault.stateFingerprint && <FingerprintDialog code={vault.stateFingerprint} onClose={() => setShowFingerprint(false)} />}
      {leaving && (
        <ConfirmDialog
          title={everywhere ? t.teams.leave.confirmEverywhere(vault.name) : t.teams.leave.confirmHere(vault.name)}
          body={t.teams.leave.body}
          confirm={t.common.remove}
          onConfirm={leave}
          onClose={() => setLeaving(false)}
        />
      )}
    </div>
  );
}

/** A new HTTPS token for a team's repository. */
export function TokenDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
  const t = useT();
  const { toast, applyTeamView } = useStore();
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!token.trim() || busy) return;
    setBusy(true);
    try {
      const view = await api.teamSetGitToken(vault.id, token.trim());
      applyTeamView(view);
      if (view.remote?.error) {
        setError(remoteErrorText(view.remote.error));
        setBusy(false);
        return;
      }
      toast({ text: t.teams.token.replaced, tone: "success" });
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t.teams.token.title}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!token.trim()}>{t.common.save}</Button>
        </>
      }
    >
      <Field label={t.teams.token.label} error={error}>
        <PasswordInput value={token} onChange={(v) => { setToken(v); setError(null); }} invalid={!!error} autoFocus />
      </Field>
    </Modal>
  );
}

/* ---------- who has which access to which record ---------- */

interface Row {
  id: string;
  name: string;
  sub?: string;
}

function AccessMatrix({ vault }: { vault: Vault }) {
  const t = useT();
  const { ws, update, toast, applyTeamView } = useStore();
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const members = ws.members.filter((m) => m.vaultId === vault.id && !m.recovery).sort(byRank);
  const mine = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId === vault.id);
  const sections: [Coll, string, Row[]][] = (
    [
      ["host", t.teams.collPlural.host, mine(ws.hosts).map((h) => ({ id: h.id, name: h.label, sub: `${h.user}@${h.address}` }))],
      ["key", t.teams.collPlural.key, mine(ws.keys).map((k) => ({ id: k.id, name: k.name }))],
      ["snippet", t.teams.collPlural.snippet, mine(ws.snippets).map((s) => ({ id: s.id, name: s.name }))],
      ["tunnel", t.teams.collPlural.tunnel, mine(ws.tunnels).map((x) => ({ id: x.id, name: x.name }))],
    ] as [Coll, string, Row[]][]
  ).filter(([, , rows]) => rows.length > 0);

  const levelOf = (record: string, member: string): Level => ws.access.find((a) => a.vaultId === vault.id && a.record === record && a.member === member)?.level ?? "none";
  const manage = canManage(vault.myRole);

  /** Levels I may set for this cell: up to my own access; only owners lower an admin's or owner's. */
  const choices = (coll: Coll, record: string, m: Member): Level[] => {
    if (!manage || m.isYou || !vault.me) return [];
    const own = levelOf(record, vault.me);
    const current = levelOf(record, m.id);
    if (!atLeast(own, current)) return [];
    const protectedMember = canManage(m.role) && vault.myRole !== "owner";
    return levelsFor(coll).filter((l) => atLeast(own, l) && (!protectedMember || atLeast(l, current)));
  };

  const apply = async (changes: { coll: Coll; record: string; member: string; level: Level }[]) => {
    if (!changes.length) return;
    const keys = changes.map((c) => `${c.record}/${c.member}`);
    setBusy((b) => new Set([...b, ...keys]));
    try {
      if (api.real) {
        applyTeamView(await api.teamAccessSet(vault.id, changes.map(({ record, member, level }) => ({ record, member, level }))));
      } else {
        await sleep(250);
        update((w) => ({
          ...w,
          access: [
            ...w.access.filter((a) => !(a.vaultId === vault.id && changes.some((c) => c.record === a.record && c.member === a.member))),
            ...changes.filter((c) => c.level !== "none").map((c) => ({ ...c, vaultId: vault.id })),
          ],
        }));
      }
      if (changes.length > 1) toast({ text: t.teams.access.changed(changes.length), tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy((b) => new Set([...b].filter((k) => !keys.includes(k))));
  };

  const bulk = (coll: Coll, rows: Row[], m: Member, level: Level) =>
    apply(rows.filter((r) => choices(coll, r.id, m).includes(level) && levelOf(r.id, m.id) !== level).map((r) => ({ coll, record: r.id, member: m.id, level })));

  if (sections.length === 0) return <Empty icon={Users} title={t.teams.access.empty} />;

  return (
    <div className="matrix-wrap">
      <table className="matrix">
        <thead>
          <tr>
            <th className="matrix-corner" />
            {members.map((m) => (
              <th key={m.id} className="matrix-member">
                <div className="matrix-member-inner" title={m.name}>
                  <Avatar member={m} size={22} />
                  <span className="ellipsis">{m.isYou ? t.common.you : m.name}</span>
                </div>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sections.map(([coll, title, rows]) => (
            <Fragment key={coll}>
              <tr className="matrix-section">
                <th scope="rowgroup">{title}</th>
                {members.map((m) => {
                  const options = levelsFor(coll).filter((l) => rows.some((r) => choices(coll, r.id, m).includes(l)));
                  return (
                    <td key={m.id}>
                      {options.length > 0 && (
                        <Menu
                          header={`${title}: ${m.name}`}
                          trigger={(toggle, open) => <button className={`matrix-bulk ${open ? "pressed" : ""}`} onClick={toggle}>{t.teams.access.all}</button>}
                          items={options.map((l) => ({ label: t.teams.level[l], onSelect: () => bulk(coll, rows, m, l) }))}
                        />
                      )}
                    </td>
                  );
                })}
              </tr>
              {rows.map((r) => (
                <tr key={r.id}>
                  <th scope="row">
                    <div className="matrix-record">
                      <span className="ellipsis strong-ish">{r.name}</span>
                      {r.sub && <span className="ellipsis mono muted small">{r.sub}</span>}
                    </div>
                  </th>
                  {members.map((m) => {
                    const level = levelOf(r.id, m.id);
                    const options = choices(coll, r.id, m);
                    const key = `${r.id}/${m.id}`;
                    return (
                      <td key={m.id}>
                        {options.length > 1 ? (
                          <select
                            className={`level-select level-${level} ${busy.has(key) ? "busy" : ""}`}
                            value={level}
                            disabled={busy.has(key)}
                            aria-label={`${r.name}: ${m.name}`}
                            onChange={(e) => apply([{ coll, record: r.id, member: m.id, level: e.target.value as Level }])}
                          >
                            {options.map((l) => <option key={l} value={l}>{t.teams.level[l]}</option>)}
                          </select>
                        ) : (
                          <span className={`level-pill level-${level}`}>{t.teams.level[level]}</span>
                        )}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function TeamsOverview() {
  const t = useT();
  const { ws, setVault, navigate, sync } = useStore();
  const [dialog, setDialog] = useState<"create" | "join" | "id" | null>(null);
  const teams = ws.vaults.filter((v) => v.kind === "team");
  const [pending, setPending] = useState<PendingTeam[]>([]);
  const joinedIds = teams.map((t) => t.id).join(",");
  // Teams joined on other devices: looked at again after every sync and every join here.
  useEffect(() => {
    if (api.real) api.teamsPending().then(setPending).catch(() => setPending([]));
  }, [sync?.syncedAt, joinedIds]);

  return (
    <div className="page">
      <PageHeader title={t.teams.overview.title}>
        <Button icon={IdCard} onClick={() => setDialog("id")}>{t.teams.overview.myId}</Button>
        <Button icon={LogIn} onClick={() => setDialog("join")}>{t.teams.overview.join}</Button>
        <Button variant="primary" icon={Plus} onClick={() => setDialog("create")}>{t.teams.overview.create}</Button>
      </PageHeader>

      {teams.length === 0 && pending.length === 0 && <Empty icon={Users} title={t.teams.overview.empty} />}

      <div className="team-cards">
        {teams.map((team) => {
          const count = ws.members.filter((m) => m.vaultId === team.id && !m.recovery).length;
          const hosts = ws.hosts.filter((h) => h.vaultId === team.id).length;
          const st = t.teams.overview.status;
          const status = team.syncStatus === "offline" ? st.folderUnavailable : team.remoteError ? st.unreachable : team.removed ? st.removed : team.joined === false ? st.waiting : team.problems?.length ? st.problems : team.syncStatus === "conflict" ? t.common.syncStatus.conflict : null;
          return (
            <button
              key={team.id}
              className="team-card"
              onClick={() => {
                setVault(team.id);
                navigate("team");
              }}
            >
              <VaultAvatar vault={team} size={36} />
              <div className="team-card-main">
                <span className="strong">{team.name}</span>
                <span className="muted small">
                  {t.teams.overview.memberCount(count)} · {t.common.hostCount(hosts)}
                </span>
              </div>
              {status && <span className={`vault-sync ${team.removed || team.problems?.length ? "conflict" : team.syncStatus === "synced" ? "offline" : team.syncStatus}`}><span className="dot" />{status}</span>}
            </button>
          );
        })}
      </div>

      {pending.length > 0 && (
        <>
          <h3 className="section-title pending-title">{t.teams.overview.onOtherDevices}</h3>
          <div className="team-cards">
            {pending.map((p) => (
              <PendingTeamCard key={p.teamId} team={p} onChanged={() => api.teamsPending().then(setPending).catch(() => {})} />
            ))}
          </div>
        </>
      )}

      {dialog === "create" && <CreateTeamDialog onClose={() => setDialog(null)} onDone={() => navigate("team")} />}
      {dialog === "join" && <JoinTeamDialog onClose={() => setDialog(null)} onDone={() => navigate("team")} />}
      {dialog === "id" && <MyIdDialog onClose={() => setDialog(null)} />}
    </div>
  );
}

/** A team joined on another device of this person: one in Git joins by itself, one in a folder needs the folder here. */
function PendingTeamCard({ team, onChanged }: { team: PendingTeam; onChanged: () => void }) {
  const t = useT();
  const { toast, applyTeamView } = useStore();
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState(false);

  const pickFolder = async () => {
    const path = await api.pickFolder();
    if (!path) return;
    setBusy(true);
    try {
      applyTeamView(await api.teamJoinLinked(team.teamId, path));
      toast({ text: t.teams.join.joined(team.name), tone: "success" });
      onChanged();
    } catch (e) {
      toast({ text: isCoreError(e) && e.kind === "notATeamVault" ? t.teams.pending.otherTeam : errorText(e), tone: "error" });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="team-card pending">
      <span className="pending-avatar"><Users size={16} /></span>
      <div className="team-card-main">
        <span className="strong">{team.name || t.teams.pending.fallbackName}</span>
        <span className={`small ${team.error ? "err-text" : "muted"}`}>
          {team.url ? (team.error ? remoteErrorText(team.error) : t.teams.pending.connecting) : t.teams.pending.notConnected}
        </span>
      </div>
      {!team.url && <Button size="sm" variant="primary" loading={busy} onClick={() => void pickFolder()}>{t.teams.pending.chooseFolder}</Button>}
      <Menu
        trigger={(open, isOpen) => <IconButton icon={Ellipsis} label={t.teams.more} className={isOpen ? "pressed" : ""} onClick={open} />}
        items={[{ label: t.teams.leave.everywhere, icon: LogOut, danger: true, onSelect: () => setRemoving(true) }]}
      />
      {removing && (
        <ConfirmDialog
          title={t.teams.pending.removeTitle(team.name)}
          confirm={t.common.remove}
          onClose={() => setRemoving(false)}
          onConfirm={async () => {
            try {
              await api.teamLeave(team.teamId);
              onChanged();
            } catch (e) {
              toast({ text: errorText(e), tone: "error" });
            }
          }}
        />
      )}
    </div>
  );
}

/** The team's state fingerprint, to read out and compare with other members after both synced. */
function FingerprintDialog({ code, onClose }: { code: string; onClose: () => void }) {
  const t = useT();
  const { toast } = useStore();
  return (
    <Modal
      title={t.teams.page.stateFingerprint}
      width={400}
      onClose={onClose}
      onSubmit={onClose}
      footer={
        <>
          <Button type="button" icon={Copy} onClick={async () => toast((await copyText(code)) ? { text: t.teams.fingerprint.copied, tone: "success" } : { text: t.common.copyFailed, tone: "error" })}>{t.common.copy}</Button>
          <Button type="submit" variant="primary">{t.common.done}</Button>
        </>
      }
    >
      <div className="pair-code mono">{code}</div>
      <div className="muted small center">{t.teams.fingerprint.hint}</div>
    </Modal>
  );
}
