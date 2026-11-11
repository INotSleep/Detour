// A group's settings: this person's own proxy and login for its hosts, and for those who manage a
// team, the proxy of the whole team.

import { useState } from "react";
import { api, errorText } from "../lib/api";
import { setGroupProxy, type ListedGroup } from "../lib/groups";
import { useStore } from "../lib/store";
import { canManage } from "../lib/teams";
import { useT } from "../i18n";
import type { Auth, GroupPrefs, Vault } from "../lib/types";
import { Button, Field, Modal, PasswordInput, Segmented } from "./ui";

/** Where this person's settings of a group are kept. */
export const groupPrefsKey = (vault: Vault, g: ListedGroup) => `${vault.id}/${g.mine ? `mine:${g.mine.id}` : `team:${g.name}`}`;

type AuthChoice = "none" | Auth["type"];

export function GroupSettingsDialog({ group, mine, onClose }: { group: ListedGroup; mine: boolean; onClose: () => void }) {
  const t = useT();
  const g = t.hosts.groups;
  const l = t.terminal.login;
  const { ws, vault, update, toast } = useStore();
  const key = groupPrefsKey(vault, group);
  const saved = ws.groupPrefs.find((p) => p.key === key);
  const personalKeys = ws.keys.filter((k) => k.vaultId === "personal");
  const personalProxies = ws.proxies.filter((p) => p.vaultId === "personal");
  const teamProxies = vault.kind === "team" ? ws.proxies.filter((p) => p.vaultId === vault.id) : [];
  // The team's proxy of a shared group, for those who manage the team.
  const teamProxyShown = vault.kind === "team" && !group.mine && !mine && canManage(vault.myRole);

  const [proxy, setProxy] = useState(saved?.proxy ? `${saved.proxy.vault}|${saved.proxy.id}` : "");
  const [teamProxy, setTeamProxy] = useState(group.record?.proxyId ?? "");
  const [user, setUser] = useState(saved?.user ?? "");
  const [auth, setAuth] = useState<AuthChoice>(saved?.auth?.type ?? "none");
  const [keyId, setKeyId] = useState(saved?.auth?.type === "key" ? saved.auth.keyId : personalKeys[0]?.id ?? "");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const keyMissing = auth === "key" && !keyId;

  const save = async () => {
    if (busy || keyMissing) return;
    const [pv, pid] = proxy.split("|");
    const next: GroupPrefs = {
      key,
      proxy: proxy ? { vault: pv!, id: pid! } : null,
      user: user.trim() || null,
      auth: auth === "none" ? null : auth === "key" ? { type: "key", keyId } : { type: auth },
      hasPassword: auth === "password" && (!!password || !!saved?.hasPassword),
      newPassword: auth === "password" && password ? password : undefined,
    };
    setBusy(true);
    try {
      const empty = !next.proxy && !next.user && !next.auth;
      const all = api.real ? await api.groupPrefsSet(next, empty) : [...ws.groupPrefs.filter((p) => p.key !== key), ...(empty ? [] : [{ ...next, newPassword: undefined }])];
      update((w) => {
        const base = { ...w, groupPrefs: all };
        return teamProxyShown && teamProxy !== (group.record?.proxyId ?? "") ? setGroupProxy(base, vault.id, group.name, teamProxy || null) : base;
      });
      toast({ text: g.saved, tone: "success" });
      onClose();
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
    setBusy(false);
  };

  return (
    <Modal
      title={g.settingsTitle(group.name)}
      width={460}
      onClose={onClose}
      onSubmit={() => void save()}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={keyMissing}>{t.common.save}</Button>
        </>
      }
    >
      <Field label={g.proxy}>
        <select className="input" value={proxy} onChange={(e) => setProxy(e.target.value)} autoFocus>
          <option value="">{g.notSet}</option>
          {personalProxies.length > 0 && (
            <optgroup label={g.yours}>
              {personalProxies.map((p) => <option key={p.id} value={`personal|${p.id}`}>{p.name}</option>)}
            </optgroup>
          )}
          {teamProxies.length > 0 && (
            <optgroup label={vault.name}>
              {teamProxies.map((p) => <option key={p.id} value={`${vault.id}|${p.id}`}>{p.name}</option>)}
            </optgroup>
          )}
        </select>
      </Field>
      {teamProxyShown && (
        <Field label={g.teamProxy}>
          <select className="input" value={teamProxy} onChange={(e) => setTeamProxy(e.target.value)}>
            <option value="">{g.notSet}</option>
            {teamProxies.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
        </Field>
      )}
      <Field label={g.user}>
        <input className="input mono" value={user} onChange={(e) => setUser(e.target.value)} autoComplete="off" spellCheck={false} />
      </Field>
      <Field label={g.login}>
        <Segmented
          value={auth}
          onChange={setAuth}
          options={[
            { value: "none", label: g.notSet },
            { value: "key", label: l.ownKey },
            { value: "password", label: t.common.password },
            { value: "agent", label: "ssh-agent" },
          ]}
        />
      </Field>
      {auth === "key" && (
        <Field label={t.common.key} error={keyMissing ? l.noKeys : null}>
          <select className="input" value={keyId} onChange={(e) => setKeyId(e.target.value)} disabled={!personalKeys.length}>
            {personalKeys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
          </select>
        </Field>
      )}
      {auth === "password" && (
        <Field label={t.common.password}>
          <PasswordInput value={password} onChange={setPassword} placeholder={saved?.hasPassword ? l.passwordSaved : l.askPassword} />
        </Field>
      )}
    </Modal>
  );
}
