import { useState } from "react";
import { Ellipsis, LogIn, Plus, RefreshCw, TriangleAlert, UserMinus, UserPlus, Users } from "lucide-react";
import { plural, sleep, uid, useStore } from "../lib/store";
import type { Member, Role, Vault } from "../lib/types";
import { Avatar, SyncIcon, VaultAvatar, roleLabel, syncStatusText, syncTarget } from "./common";
import { Button, ConfirmDialog, Empty, IconButton, Menu, PageHeader } from "./ui";
import { CreateTeamDialog, InviteDialog, JoinTeamDialog } from "./TeamDialogs";

export function TeamsView() {
  const { vault } = useStore();
  return vault.kind === "team" ? <TeamPage vault={vault} /> : <TeamsOverview />;
}

function TeamPage({ vault }: { vault: Vault }) {
  const { ws, update, toast, resolveConflicts } = useStore();
  const [inviting, setInviting] = useState(false);
  const [removing, setRemoving] = useState<Member | null>(null);

  const members = ws.members.filter((m) => m.vaultId === vault.id);
  const activity = ws.activity.filter((a) => a.vaultId === vault.id);
  const me = members.find((m) => m.isYou);
  const canManage = me?.role === "owner" || me?.role === "admin";

  const setVaultState = (patch: Partial<Vault>) => update((w) => ({ ...w, vaults: w.vaults.map((v) => (v.id === vault.id ? { ...v, ...patch } : v)) }));

  const sync = async () => {
    const hadConflicts = vault.conflicts.length > 0;
    setVaultState({ syncStatus: "syncing" });
    await sleep(1300);
    if (hadConflicts) {
      setVaultState({ syncStatus: "conflict", lastSync: "только что" });
      toast({ text: "Сначала решите конфликт", tone: "error", action: { label: "Решить", run: () => resolveConflicts(vault.id) } });
      return;
    }
    setVaultState({ syncStatus: "synced", lastSync: "только что" });
    toast({ text: "Синхронизировано", tone: "success" });
  };

  const setRole = (m: Member, role: Role) => {
    update((w) => ({ ...w, members: w.members.map((x) => (x.id === m.id ? { ...x, role } : x)) }));
    toast({ text: `${m.name}: ${roleLabel[role].toLowerCase()}`, tone: "success" });
  };

  const revoke = (m: Member) => {
    update((w) => ({ ...w, members: w.members.filter((x) => x.id !== m.id) }));
    toast({ text: `Приглашение для ${m.email} отозвано`, action: { label: "Вернуть", run: () => update((w) => ({ ...w, members: [...w.members, m] })) } });
  };

  const removeMember = (m: Member) => {
    update((w) => ({
      ...w,
      members: w.members.filter((x) => x.id !== m.id),
      activity: [{ id: uid("a"), vaultId: vault.id, memberId: me!.id, action: "удалили", target: m.name, when: "только что" }, ...w.activity],
    }));
    toast({ text: `${m.name} удалён из команды`, tone: "success" });
  };

  const syncing = vault.syncStatus === "syncing";

  return (
    <div className="page">
      <PageHeader title={vault.name}>
        <Button icon={RefreshCw} loading={syncing} onClick={sync}>{syncing ? "Синхронизация" : "Синхронизировать"}</Button>
        {canManage && <Button variant="primary" icon={UserPlus} onClick={() => setInviting(true)}>Пригласить</Button>}
      </PageHeader>

      <div className="sync-line">
        <SyncIcon sync={vault.sync} size={14} />
        <span className="mono">{syncTarget(vault.sync)}</span>
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
        {vault.syncStatus !== "syncing" && <span className="muted">· {vault.lastSync}</span>}
      </div>

      <div className="team-grid">
        <section>
          <h3 className="section-title">
            Участники <span className="count">{members.length}</span>
          </h3>
          <div className="rows">
            {members.map((m) => {
              const editable = canManage && !m.isYou && m.role !== "owner" && m.status === "active";
              return (
                <div className="member-row" key={m.id}>
                  <Avatar member={m} size={32} />
                  <div className="member-main">
                    <span className="member-name">
                      {m.status === "invited" ? m.email : m.name}
                      {m.isYou && <span className="chip">вы</span>}
                    </span>
                    <span className="member-sub">{m.status === "invited" ? "Ожидает" : m.email}</span>
                  </div>
                  {editable ? (
                    <select className="input input-sm role-select" value={m.role} onChange={(e) => setRole(m, e.target.value as Role)}>
                      <option value="admin">Админ</option>
                      <option value="member">Участник</option>
                      <option value="readOnly">Только чтение</option>
                    </select>
                  ) : (
                    <span className="member-role">{roleLabel[m.role]}</span>
                  )}
                  <div className="member-actions">
                    {m.status === "invited" && canManage && <Button size="sm" variant="ghost" onClick={() => revoke(m)}>Отозвать</Button>}
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
              {activity.map((a) => {
                const who = ws.members.find((m) => m.id === a.memberId);
                return (
                  <div className="feed-item" key={a.id}>
                    {who && <Avatar member={who} size={22} />}
                    <div>
                      <div>
                        <span className="strong">{who?.isYou ? "Вы" : who?.name.split(" ")[0]}</span> {a.action} <span className="strong">{a.target}</span>
                      </div>
                      <div className="muted small">{a.when}</div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </section>
      </div>

      {inviting && <InviteDialog vault={vault} onClose={() => setInviting(false)} />}
      {removing && (
        <ConfirmDialog
          title={`Удалить ${removing.name}?`}
          body={`Доступ к «${vault.name}» будет закрыт.`}
          confirm="Удалить"
          onConfirm={() => removeMember(removing)}
          onClose={() => setRemoving(null)}
        />
      )}
    </div>
  );
}

function TeamsOverview() {
  const { ws, setVault, navigate } = useStore();
  const [dialog, setDialog] = useState<"create" | "join" | null>(null);
  const teams = ws.vaults.filter((v) => v.kind === "team");

  return (
    <div className="page">
      <PageHeader title="Команды">
        <Button icon={LogIn} onClick={() => setDialog("join")}>Вступить по коду</Button>
        <Button variant="primary" icon={Plus} onClick={() => setDialog("create")}>Создать команду</Button>
      </PageHeader>

      {teams.length === 0 && <Empty icon={Users} title="Вы пока не в команде" />}

      <div className="team-cards">
        {teams.map((t) => {
          const count = ws.members.filter((m) => m.vaultId === t.id).length;
          const hosts = ws.hosts.filter((h) => h.vaultId === t.id).length;
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
              {t.syncStatus !== "synced" && (
                <span className={`vault-sync ${t.syncStatus}`}>
                  <span className="dot" />
                  {syncStatusText[t.syncStatus]}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {dialog === "create" && <CreateTeamDialog onClose={() => setDialog(null)} />}
      {dialog === "join" && <JoinTeamDialog onClose={() => setDialog(null)} />}
    </div>
  );
}
