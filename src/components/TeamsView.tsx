import { Fragment, useState } from "react";
import { ChevronRight, Copy, Ellipsis, IdCard, LogIn, LogOut, Plus, RefreshCw, ShieldAlert, ShieldQuestion, TriangleAlert, UserMinus, UserPlus, Users, UserX } from "lucide-react";
import { api, errorText, type CoreError } from "../lib/api";
import { copyText, formatAgo, plural, sleep, useStore } from "../lib/store";
import { atLeast, canManage, removeVault } from "../lib/teams";
import type { ActivityItem, Coll, Level, Member, Problem, Role, Vault } from "../lib/types";
import { Avatar, SyncIcon, VaultAvatar, roleLabel, syncStatusText, syncTarget } from "./common";
import { Button, ConfirmDialog, Empty, Field, IconButton, Menu, Modal, PageHeader, PasswordInput, Segmented } from "./ui";
import { AddMemberDialog, CreateTeamDialog, JoinTeamDialog, MyIdDialog, useMyId } from "./TeamDialogs";

export function TeamsView() {
  const { vault } = useStore();
  return vault.kind === "team" ? <TeamPage vault={vault} /> : <TeamsOverview />;
}

const collName: Record<Coll, string> = { host: "Хост", key: "Ключ", snippet: "Сниппет", tunnel: "Туннель" };

export const levelLabel: Record<Level, string> = { none: "Нет", view: "Видит", secret: "Секрет", edit: "Изменяет" };

/** Levels a record of this kind can have. */
const levelsFor = (coll: Coll): Level[] => (coll === "host" || coll === "key" ? ["none", "view", "secret", "edit"] : ["none", "view", "edit"]);

function activityText(a: ActivityItem): string {
  switch (a.kind) {
    case "created": return "Команда создана";
    case "renamed": return `Команда переименована в «${a.name}»`;
    case "memberAdded": return `${a.member} добавлен в команду`;
    case "memberChanged": return `${a.member}: роль изменена`;
    case "memberRemoved": return `${a.member} удалён из команды`;
    case "accessChanged": return `${a.member}: ${levelLabel[a.level].toLowerCase()} · «${a.name}»`;
    case "secretChanged": return `${a.coll === "key" ? "Ключ" : "Пароль"} «${a.name}» изменён`;
    default: {
      const verb = a.kind === "added" ? "добавлен" : a.kind === "changed" ? "изменён" : "удалён";
      return `${collName[a.coll]} «${a.name}» ${verb}`;
    }
  }
}

/** Why a team's repository could not be reached, in words. */
export function remoteErrorText(e: CoreError): string {
  switch (e.kind) {
    case "hostKeyChanged": return "Ключ сервера репозитория изменился";
    case "unreachable": case "connect": case "io": return "Нет связи с репозиторием";
    case "authFailed": return "Репозиторий не принял ключ";
    case "needSecret": return "Ключ для репозитория защищён паролем: сохраните пароль ключа";
    default: return errorText(e);
  }
}

export function problemText(p: Problem): string {
  const who = p.kind === "unknownFile" ? "" : p.authorName || `устройство ${p.device.slice(0, 8)}`;
  switch (p.kind) {
    case "rejected": return `Отклонено изменение: ${who}`;
    case "missing": return `Хранилище не показывает изменение: ${who}`;
    case "forked": return `Две версии одного изменения: ${who}`;
    case "rolledBack": return `Хранилище откатило изменения: ${who}`;
    case "unknownFile": return `Посторонний файл: ${p.path}`;
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
  const { ws, update, toast, applyTeamView, resolveConflicts, setVault } = useStore();
  const me = useMyId();
  const [tab, setTab] = useState<"members" | "access">("members");
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<Member | null>(null);
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
        } else toast(view.conflicts.length ? { text: "Есть конфликт", tone: "error", action: { label: "Решить", run: () => resolveConflicts(vault.id) } } : { text: "Синхронизировано", tone: "success" });
      } else {
        await sleep(900);
        update((w) => ({ ...w, vaults: w.vaults.map((v) => (v.id === vault.id ? { ...v, lastSync: Date.now() } : v)) }));
        toast({ text: "Синхронизировано", tone: "success" });
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
      toast(view.remote?.error ? { text: remoteErrorText(view.remote.error), tone: "error" } : { text: "Ключ сервера заменён", tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setSyncing(false);
  };

  const changeRole = async (m: Member, role: Role) => {
    try {
      if (api.real) applyTeamView(await api.teamMemberUpdate(vault.id, m.id, role));
      else update((w) => ({ ...w, members: w.members.map((x) => (x.id === m.id && x.vaultId === vault.id ? { ...x, role } : x)) }));
      toast({ text: `${m.name}: ${roleLabel[role].toLowerCase()}`, tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const removeMember = async (m: Member) => {
    try {
      if (api.real) applyTeamView(await api.teamMemberRemove(vault.id, m.id));
      else update((w) => ({ ...w, members: w.members.filter((x) => !(x.id === m.id && x.vaultId === vault.id)), activity: [...w.activity, { vaultId: vault.id, authorName: "Вы", isYou: true, at: Date.now(), kind: "memberRemoved", member: m.name }] }));
      toast({ text: `${m.name} удалён из команды`, tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const leave = async () => {
    try {
      if (api.real) await api.teamLeave(vault.id);
      setVault("personal");
      update((w) => removeVault(w, vault.id));
      toast({ text: `«${vault.name}» убрана с этого устройства`, tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const copyMyId = async () => me && (await copyText(me.detourId)) && toast({ text: "Detour ID скопирован", tone: "success" });

  return (
    <div className={`page ${tab === "access" ? "wide" : ""}`}>
      <PageHeader title={vault.name}>
        <Button icon={RefreshCw} loading={syncing} onClick={() => sync()}>{syncing ? "Синхронизация" : "Синхронизировать"}</Button>
        {manage && <Button variant="primary" icon={UserPlus} onClick={() => setAdding(true)}>Добавить участника</Button>}
        <Menu
          trigger={(toggle, open) => <IconButton icon={Ellipsis} label="Ещё" className={open ? "pressed" : ""} onClick={toggle} />}
          items={[{ label: "Убрать с этого устройства", icon: LogOut, danger: true, onSelect: () => setLeaving(true) }]}
        />
      </PageHeader>

      <div className="sync-line">
        <SyncIcon sync={vault.sync} />
        <span className="mono ellipsis sync-path" title={target}>{target}</span>
        <span className="sep" />
        {vault.syncStatus === "conflict" ? (
          <button className="chip warn chip-btn" onClick={() => resolveConflicts(vault.id)}>
            <TriangleAlert size={12} /> Конфликт · решить
          </button>
        ) : vault.remoteError ? (
          <span className="vault-sync conflict">
            <span className="dot" />
            Нет связи
          </span>
        ) : (
          <span className={`vault-sync ${vault.syncStatus}`}>
            <span className="dot" />
            {syncStatusText[vault.syncStatus]}
          </span>
        )}
        {!offline && !vault.remoteError && vault.lastSync && <span className="muted">· {formatAgo(vault.lastSync)}</span>}
        {vault.pending && <span className="chip">Не отправлено</span>}
      </div>

      {newServerKey && (
        <div className="banner info">
          <ShieldQuestion size={16} />
          <span className="grow">Новый ключ сервера репозитория: <span className="mono">{newServerKey}</span></span>
          <Button size="sm" variant="primary" loading={syncing} onClick={() => sync(newServerKey)}>Доверять</Button>
        </div>
      )}

      {vault.remoteError?.kind === "hostKeyChanged" && (
        <div className="banner warn">
          <ShieldAlert size={16} />
          <span className="grow">
            Ключ сервера репозитория изменился
            <span className="banner-sub mono">Был {vault.remoteError.expected}</span>
            <span className="banner-sub mono">Сейчас {vault.remoteError.actual}</span>
          </span>
          <Button size="sm" variant="ghost" className="danger-text" loading={syncing} onClick={() => replaceKey(vault.remoteError?.kind === "hostKeyChanged" ? vault.remoteError.actual : "")}>Заменить ключ</Button>
        </div>
      )}

      {vault.remoteError && !newServerKey && vault.remoteError.kind !== "hostKeyChanged" && (
        <div className="banner warn">
          <TriangleAlert size={16} />
          <span className="grow">{remoteErrorText(vault.remoteError)}</span>
          {tokenRejected ? (
            <Button size="sm" variant="primary" onClick={() => setTokenDialog(true)}>Новый токен</Button>
          ) : (
            <Button size="sm" loading={syncing} onClick={() => sync()}>Повторить</Button>
          )}
        </div>
      )}

      {offline && (
        <div className="banner warn">
          <TriangleAlert size={16} />
          <span className="grow">Папка недоступна</span>
          <Button size="sm" loading={syncing} onClick={() => sync()}>Повторить</Button>
        </div>
      )}

      {!offline && vault.removed && (
        <div className="banner warn">
          <UserX size={16} />
          <span className="grow">Вас удалили из команды</span>
          <Button size="sm" onClick={leave}>Убрать с устройства</Button>
        </div>
      )}

      {!offline && vault.joined === false && !vault.removed && (
        <div className="banner info">
          <IdCard size={16} />
          <span className="grow">Вас ещё не добавили. Отправьте свой Detour ID админу команды.</span>
          <Button size="sm" icon={Copy} onClick={copyMyId}>Копировать ID</Button>
          <Button size="sm" variant="primary" loading={syncing} onClick={() => sync()}>Проверить</Button>
        </div>
      )}

      {problems.length > 0 && (
        <div className="banner warn problems">
          <button className="problems-head" onClick={() => setShowProblems((s) => !s)} aria-expanded={showProblems}>
            <ShieldAlert size={16} />
            <span className="grow">
              {problems.length} {plural(problems.length, "проблема", "проблемы", "проблем")} с хранилищем
            </span>
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
            <Segmented value={tab} onChange={setTab} options={[{ value: "members", label: "Участники" }, { value: "access", label: "Доступ" }]} />
          </div>

          {tab === "access" ? (
            <AccessMatrix vault={vault} />
          ) : (
            <div className="team-grid">
              <section>
                <div className="member-head">
                  <h3 className="section-title">
                    Участники <span className="count">{members.length}</span>
                  </h3>
                  <span className="section-title">Роль</span>
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
                            {m.recovery ? "Ключ восстановления" : m.name}
                            {m.isYou && <span className="chip">вы</span>}
                          </span>
                          <span className="member-sub mono">{m.fingerprint}</span>
                        </div>
                        {editable ? (
                          <select className="input input-sm role-select" value={m.role} onChange={(e) => changeRole(m, e.target.value as Role)}>
                            {vault.myRole === "owner" && <option value="owner">Владелец</option>}
                            {vault.myRole === "owner" && <option value="admin">Админ</option>}
                            <option value="member">Участник</option>
                          </select>
                        ) : (
                          <span className="member-role">{roleLabel[m.role]}</span>
                        )}
                        <div className="member-actions">
                          {editable && (
                            <Menu
                              trigger={(toggle, open) => <IconButton icon={Ellipsis} label="Ещё" className={open ? "pressed" : ""} onClick={toggle} />}
                              items={[{ label: "Удалить из команды", icon: UserMinus, danger: true, onSelect: () => setRemoving(m) }]}
                            />
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </section>

              <section>
                <h3 className="section-title">Активность</h3>
                {activity.length === 0 ? (
                  <div className="muted small">Пока пусто</div>
                ) : (
                  <div className="feed">
                    {activity.slice(0, 30).map((a, i) => (
                      <div className="feed-item" key={i}>
                        <div>
                          <div className="strong-ish">{activityText(a)}</div>
                          <div className="muted small">{a.isYou ? "Вы" : a.authorName} · {formatAgo(a.at)}</div>
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
          title={`Удалить ${removing.name}?`}
          body="Новые изменения команды станут для него недоступны."
          confirm="Удалить"
          onConfirm={() => removeMember(removing)}
          onClose={() => setRemoving(null)}
        />
      )}
      {leaving && (
        <ConfirmDialog
          title={`Убрать «${vault.name}» с этого устройства?`}
          body="Хранилище команды и ваше участие останутся, подключить снова можно в любой момент."
          confirm="Убрать"
          onConfirm={leave}
          onClose={() => setLeaving(false)}
        />
      )}
    </div>
  );
}

/** A new HTTPS token for a team's repository. */
export function TokenDialog({ vault, onClose }: { vault: Vault; onClose: () => void }) {
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
      toast({ text: "Токен заменён", tone: "success" });
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title="Токен доступа к репозиторию"
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!token.trim()}>Сохранить</Button>
        </>
      }
    >
      <Field label="Новый токен" error={error}>
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
  const { ws, update, toast, applyTeamView } = useStore();
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const members = ws.members.filter((m) => m.vaultId === vault.id && !m.recovery).sort(byRank);
  const mine = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId === vault.id);
  const sections: [Coll, string, Row[]][] = (
    [
      ["host", "Хосты", mine(ws.hosts).map((h) => ({ id: h.id, name: h.label, sub: `${h.user}@${h.address}` }))],
      ["key", "Ключи", mine(ws.keys).map((k) => ({ id: k.id, name: k.name }))],
      ["snippet", "Сниппеты", mine(ws.snippets).map((s) => ({ id: s.id, name: s.name }))],
      ["tunnel", "Туннели", mine(ws.tunnels).map((t) => ({ id: t.id, name: t.name }))],
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
      if (changes.length > 1) toast({ text: `Доступ изменён: ${changes.length}`, tone: "success" });
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy((b) => new Set([...b].filter((k) => !keys.includes(k))));
  };

  const bulk = (coll: Coll, rows: Row[], m: Member, level: Level) =>
    apply(rows.filter((r) => choices(coll, r.id, m).includes(level) && levelOf(r.id, m.id) !== level).map((r) => ({ coll, record: r.id, member: m.id, level })));

  if (sections.length === 0) return <Empty icon={Users} title="Нет записей" />;

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
                  <span className="ellipsis">{m.isYou ? "Вы" : m.name}</span>
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
                          trigger={(toggle, open) => <button className={`matrix-bulk ${open ? "pressed" : ""}`} onClick={toggle}>Всем</button>}
                          items={options.map((l) => ({ label: levelLabel[l], onSelect: () => bulk(coll, rows, m, l) }))}
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
                            {options.map((l) => <option key={l} value={l}>{levelLabel[l]}</option>)}
                          </select>
                        ) : (
                          <span className={`level-pill level-${level}`}>{levelLabel[level]}</span>
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
  const { ws, setVault, navigate } = useStore();
  const [dialog, setDialog] = useState<"create" | "join" | "id" | null>(null);
  const teams = ws.vaults.filter((v) => v.kind === "team");

  return (
    <div className="page">
      <PageHeader title="Команды">
        <Button icon={IdCard} onClick={() => setDialog("id")}>Мой Detour ID</Button>
        <Button icon={LogIn} onClick={() => setDialog("join")}>Подключить</Button>
        <Button variant="primary" icon={Plus} onClick={() => setDialog("create")}>Создать команду</Button>
      </PageHeader>

      {teams.length === 0 && <Empty icon={Users} title="Вы пока не в команде" />}

      <div className="team-cards">
        {teams.map((t) => {
          const count = ws.members.filter((m) => m.vaultId === t.id && !m.recovery).length;
          const hosts = ws.hosts.filter((h) => h.vaultId === t.id).length;
          const status = t.syncStatus === "offline" ? "Папка недоступна" : t.remoteError ? "Нет связи с репозиторием" : t.removed ? "Вы удалены" : t.joined === false ? "Ждёт добавления" : t.problems?.length ? "Проблемы с хранилищем" : t.syncStatus === "conflict" ? "Конфликт" : null;
          return (
            <button
              key={t.id}
              className="team-card"
              onClick={() => {
                setVault(t.id);
                navigate("team");
              }}
            >
              <VaultAvatar vault={t} size={36} />
              <div className="team-card-main">
                <span className="strong">{t.name}</span>
                <span className="muted small">
                  {count} {plural(count, "участник", "участника", "участников")} · {hosts} {plural(hosts, "хост", "хоста", "хостов")}
                </span>
              </div>
              {status && <span className={`vault-sync ${t.removed || t.problems?.length ? "conflict" : t.syncStatus === "synced" ? "offline" : t.syncStatus}`}><span className="dot" />{status}</span>}
            </button>
          );
        })}
      </div>

      {dialog === "create" && <CreateTeamDialog onClose={() => setDialog(null)} onDone={() => navigate("team")} />}
      {dialog === "join" && <JoinTeamDialog onClose={() => setDialog(null)} onDone={() => navigate("team")} />}
      {dialog === "id" && <MyIdDialog onClose={() => setDialog(null)} />}
    </div>
  );
}
