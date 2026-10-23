import { useState } from "react";
import { Copy, Ellipsis, FolderSync, IdCard, LogIn, LogOut, Plus, RefreshCw, ShieldAlert, TriangleAlert, UserMinus, UserPlus, Users, UserX } from "lucide-react";
import { api, errorText } from "../lib/api";
import { copyText, formatAgo, plural, sleep, useStore } from "../lib/store";
import { canManage, removeVault } from "../lib/teams";
import type { ActivityItem, Coll, Member, Role, Vault, Workspace } from "../lib/types";
import { Avatar, VaultAvatar, roleLabel, syncStatusText } from "./common";
import { Button, ConfirmDialog, Empty, IconButton, Menu, PageHeader, Switch } from "./ui";
import { AddMemberDialog, CreateTeamDialog, JoinTeamDialog, MyIdDialog, useMyId } from "./TeamDialogs";

export function TeamsView() {
  const { vault } = useStore();
  return vault.kind === "team" ? <TeamPage vault={vault} /> : <TeamsOverview />;
}

const collName: Record<Coll, [string, string]> = {
  host: ["Хост", "Хосты"],
  key: ["Ключ", "Ключи"],
  snippet: ["Сниппет", "Сниппеты"],
  tunnel: ["Туннель", "Туннели"],
};

/** Display name of a record for the activity feed. */
function recordLabel(ws: Workspace, coll: Coll, id: string): string | null {
  const list = coll === "host" ? ws.hosts : coll === "key" ? ws.keys : coll === "snippet" ? ws.snippets : ws.tunnels;
  const r = list.find((x) => x.id === id) as { label?: string; name?: string } | undefined;
  return r?.label ?? r?.name ?? null;
}

function activityText(ws: Workspace, a: ActivityItem): string {
  switch (a.kind) {
    case "created": return "Команда создана";
    case "renamed": return `Команда переименована в «${a.name}»`;
    case "memberAdded": return `${a.member} добавлен в команду`;
    case "memberChanged": return `Права ${a.member} изменены`;
    case "memberRemoved": return `${a.member} удалён из команды`;
    default: {
      const label = recordLabel(ws, a.coll, a.id);
      const what = collName[a.coll][0] + (label ? ` «${label}»` : "");
      const verb = a.kind === "added" ? "добавлен" : a.kind === "changed" ? "изменён" : "удалён";
      return `${what} ${verb}`;
    }
  }
}

/** Which members may I change? Owners manage everyone; admins manage members and read-only users. */
function canEdit(me: Role | undefined, m: Member) {
  if (m.isYou || m.recovery || !canManage(me)) return false;
  return me === "owner" || !canManage(m.role);
}

function TeamPage({ vault }: { vault: Vault }) {
  const { ws, update, toast, applyTeamView, resolveConflicts, setVault } = useStore();
  const me = useMyId();
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<Member | null>(null);
  const [leaving, setLeaving] = useState(false);
  const [syncing, setSyncing] = useState(false);

  const rank = (m: Member) => (m.recovery ? 9 : m.isYou ? 0 : { owner: 1, admin: 2, member: 3, readOnly: 4 }[m.role]);
  const members = ws.members.filter((m) => m.vaultId === vault.id).sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
  const activity = ws.activity.filter((a) => a.vaultId === vault.id).sort((a, b) => b.at - a.at);
  const manage = !!vault.joined && canManage(vault.myRole);
  const offline = vault.syncStatus === "offline";
  const path = vault.sync.type === "folder" ? vault.sync.path : "";

  const sync = async () => {
    setSyncing(true);
    try {
      if (api.real) {
        const view = await api.teamSync(vault.id);
        applyTeamView(view);
        toast(view.conflicts.length ? { text: "Есть конфликт", tone: "error", action: { label: "Решить", run: () => resolveConflicts(vault.id) } } : { text: "Синхронизировано", tone: "success" });
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

  const changeMember = async (m: Member, role: Role, canReveal: boolean) => {
    try {
      if (api.real) applyTeamView(await api.teamMemberUpdate(vault.id, m.id, role, canReveal));
      else update((w) => ({ ...w, members: w.members.map((x) => (x.id === m.id && x.vaultId === vault.id ? { ...x, role, canReveal } : x)) }));
      toast({ text: role !== m.role ? `${m.name}: ${roleLabel[role].toLowerCase()}` : canReveal ? `${m.name} видит секреты` : `${m.name} больше не видит секреты`, tone: "success" });
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
    <div className="page">
      <PageHeader title={vault.name}>
        <Button icon={RefreshCw} loading={syncing} onClick={sync}>{syncing ? "Синхронизация" : "Синхронизировать"}</Button>
        {manage && <Button variant="primary" icon={UserPlus} onClick={() => setAdding(true)}>Добавить участника</Button>}
        <Menu
          trigger={(toggle, open) => <IconButton icon={Ellipsis} label="Ещё" className={open ? "pressed" : ""} onClick={toggle} />}
          items={[{ label: "Убрать с этого устройства", icon: LogOut, danger: true, onSelect: () => setLeaving(true) }]}
        />
      </PageHeader>

      <div className="sync-line">
        <FolderSync size={14} />
        <span className="mono ellipsis sync-path" title={path}>{path}</span>
        <span className="sep" />
        {vault.syncStatus === "conflict" ? (
          <button className="chip warn chip-btn" onClick={() => resolveConflicts(vault.id)}>
            <TriangleAlert size={12} /> Конфликт · решить
          </button>
        ) : (
          <span className={`vault-sync ${vault.syncStatus}`}>
            <span className="dot" />
            {syncStatusText[vault.syncStatus]}
          </span>
        )}
        {!offline && vault.lastSync && <span className="muted">· {formatAgo(vault.lastSync)}</span>}
      </div>

      {offline && (
        <div className="banner warn">
          <TriangleAlert size={16} />
          <span className="grow">Папка недоступна</span>
          <Button size="sm" loading={syncing} onClick={sync}>Повторить</Button>
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
          <Button size="sm" variant="primary" loading={syncing} onClick={sync}>Проверить</Button>
        </div>
      )}

      {!!vault.problems && (
        <div className="banner warn">
          <ShieldAlert size={16} />
          <span className="grow">Отклонено изменений: {vault.problems}. Подпись или права автора не прошли проверку.</span>
        </div>
      )}

      {vault.joined !== false && !offline && (
        <div className="team-grid">
          <section>
            <div className="member-head">
              <h3 className="section-title">
                Участники <span className="count">{members.length}</span>
              </h3>
              <span className="section-title">Роль</span>
              <span className="section-title">Секреты</span>
              <span />
            </div>
            <div className="rows">
              {members.map((m) => {
                const editable = canEdit(vault.myRole, m);
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
                      <select className="input input-sm role-select" value={m.role} onChange={(e) => changeMember(m, e.target.value as Role, m.canReveal)}>
                        {vault.myRole === "owner" && <option value="owner">Владелец</option>}
                        {vault.myRole === "owner" && <option value="admin">Админ</option>}
                        <option value="member">Участник</option>
                        <option value="readOnly">Только чтение</option>
                      </select>
                    ) : (
                      <span className="member-role">{roleLabel[m.role]}</span>
                    )}
                    <label className={`reveal ${editable ? "" : "static"}`} data-tip={m.canReveal ? "Видит секреты" : "Не видит секреты"}>
                      {editable ? <Switch label="Видит секреты" checked={m.canReveal} onChange={(v) => changeMember(m, m.role, v)} /> : <span className={`reveal-dot ${m.canReveal ? "on" : ""}`} />}
                    </label>
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
                      <div className="strong-ish">{activityText(ws, a)}</div>
                      <div className="muted small">{a.isYou ? "Вы" : a.authorName} · {formatAgo(a.at)}</div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>
        </div>
      )}

      {adding && <AddMemberDialog vault={vault} onClose={() => setAdding(false)} />}
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
          const status = t.syncStatus === "offline" ? "Папка недоступна" : t.removed ? "Вы удалены" : t.joined === false ? "Ждёт добавления" : t.syncStatus === "conflict" ? "Конфликт" : null;
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
              {status && <span className={`vault-sync ${t.removed ? "conflict" : t.syncStatus === "synced" ? "offline" : t.syncStatus}`}><span className="dot" />{status}</span>}
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
