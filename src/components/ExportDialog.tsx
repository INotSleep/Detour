import { useEffect, useMemo, useState } from "react";
import { ArrowRight, Check, Copy, Save } from "lucide-react";
import { api, errorText, type FileCounts, type FileInclude } from "../lib/api";
import { copyText, useStore } from "../lib/store";
import { keyFileName, keysOf, sshConfig, withJumps } from "../lib/sshConfig";
import type { Host } from "../lib/types";
import { asLoggedIn, loginUser } from "../lib/groups";
import { Button, Checkbox, Field, Modal, PasswordInput, Segmented } from "./ui";
import { useT } from "../i18n";

const INCLUDE: (keyof FileInclude)[] = ["passwords", "keys", "proxies", "knownHosts", "groups"];

/** Hosts as ~/.ssh/config entries, copied or saved to a file, with their keys if asked;
 *  or as a Detour file sealed with a password, with what logs in to them. */
export function ExportDialog({ hosts, initial, onClose }: { hosts: Host[]; initial?: string[]; onClose: () => void }) {
  const t = useT();
  const { ws, toast } = useStore();
  const [chosen, setChosen] = useState<Set<string>>(new Set(initial ?? hosts.map((h) => h.id)));
  const [withKeys, setWithKeys] = useState(false);
  const [busy, setBusy] = useState(false);
  const [format, setFormat] = useState<"ssh" | "file">("ssh");
  const [include, setInclude] = useState<FileInclude>({ passwords: true, keys: true, proxies: true, knownHosts: true, groups: true });
  const [counts, setCounts] = useState<FileCounts | null>(null);
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [tried, setTried] = useState(false);

  // Hosts as they log in (a group's or this person's own login), as the core's export has them.
  const loggedIn = useMemo(() => ws.hosts.map((h) => asLoggedIn(ws, h)), [ws]);
  const picked = hosts.filter((h) => chosen.has(h.id)).map((h) => asLoggedIn(ws, h));
  const keys = useMemo(() => keysOf(withJumps(picked, loggedIn), ws.keys), [picked, loggedIn, ws.keys]);
  // The app's export is the core's, the same as `detour hosts export`; the demo makes its own.
  const ids = picked.map((h) => h.id).join(",");
  const demoText = useMemo(() => (!api.real && picked.length ? sshConfig(picked, loggedIn, ws.keys, ws.proxies) : ""), [picked, loggedIn, ws.keys, ws.proxies]);
  const [coreText, setCoreText] = useState("");
  useEffect(() => {
    if (!api.real || !ids) return setCoreText("");
    let live = true;
    api.hostsExport(hosts[0]!.vaultId, ids.split(","), null).then((r) => live && setCoreText(r.config)).catch(() => {});
    return () => void (live = false);
  }, [ids, hosts]);
  const text = api.real ? coreText : demoText;
  useEffect(() => {
    if (format !== "file" || !ids) return setCounts(null);
    if (!api.real) {
      const withJ = withJumps(picked, loggedIn);
      const distinct = (xs: (string | undefined | null)[]) => new Set(xs.filter(Boolean)).size;
      return setCounts({ hosts: withJ.length, passwords: withJ.filter((h) => h.hasPassword).length, keys: keys.length, proxies: distinct(withJ.map((h) => h.proxyId)), knownHosts: withJ.length, groups: distinct(withJ.map((h) => h.group)) });
    }
    let live = true;
    api.hostsFileCounts(hosts[0]!.vaultId, ids.split(",")).then((c) => live && setCounts(c)).catch(() => {});
    return () => void (live = false);
  }, [format, ids, hosts]);
  const passwordError = !tried ? null : !password ? t.hosts.export.passwordRequired : repeat !== password ? t.hosts.export.passwordsDiffer : null;
  const countOf: Record<keyof FileInclude, (n: number) => string> = {
    passwords: t.hosts.export.passwords,
    keys: t.hosts.export.keys,
    proxies: t.hosts.export.proxies,
    knownHosts: t.hosts.export.serverKeys,
    groups: t.hosts.export.groups,
  };
  const all = chosen.size === hosts.length;
  const saveKeys = withKeys && keys.length > 0;

  const toggle = (id: string, on: boolean) =>
    setChosen((s) => {
      const next = new Set(s);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const copy = async () => {
    const ok = await copyText(text);
    toast(ok ? { text: t.hosts.export.copied, tone: "success" } : { text: t.common.copyFailed, tone: "error" });
    if (ok) onClose();
  };

  const save = async () => {
    if (!picked.length || busy) return;
    setBusy(true);
    try {
      const path = await api.pickSavePath("detour.conf");
      if (!path) return setBusy(false);
      const dir = path.slice(0, Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
      if (api.real) {
        const r = await api.hostsExport(picked[0]!.vaultId, picked.map((h) => h.id), saveKeys ? dir : null);
        await api.writeTextFile(path, r.config);
      } else {
        const paths: Record<string, string> = {};
        if (saveKeys) {
          const where = await api.keyFilesSave(dir, keys.map((key) => ({ key, name: keyFileName(key.name) })));
          keys.forEach((k, i) => (paths[k.id] = where[i]));
        }
        await api.writeTextFile(path, sshConfig(picked, loggedIn, ws.keys, ws.proxies, paths));
      }
      toast({ text: saveKeys ? t.hosts.export.savedWithKeys(path, t.common.keyCount(keys.length)) : t.hosts.export.saved(path), tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const saveFile = async () => {
    setTried(true);
    if (!picked.length || busy || !password || repeat !== password) return;
    setBusy(true);
    try {
      const path = await api.pickSavePath("hosts.detour");
      if (!path) return setBusy(false);
      if (api.real) await api.hostsFileExport(picked[0]!.vaultId, picked.map((h) => h.id), include, password, path);
      toast({ text: t.hosts.export.saved(path), tone: "success" });
      onClose();
    } catch (e) {
      setBusy(false);
      toast({ text: errorText(e), tone: "error" });
    }
  };

  return (
    <Modal
      title={t.hosts.export.title}
      width={600}
      onClose={onClose}
      onSubmit={format === "ssh" ? save : saveFile}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          {format === "ssh" && !saveKeys && <Button type="button" icon={Copy} disabled={!picked.length} onClick={copy}>{t.hosts.export.copy}</Button>}
          <Button type="submit" variant="primary" icon={Save} loading={busy} disabled={!picked.length}>{t.hosts.export.save}</Button>
        </>
      }
    >
      <Segmented
        value={format}
        onChange={setFormat}
        options={[
          { value: "ssh", label: t.hosts.export.sshConfig },
          { value: "file", label: t.hosts.export.file },
        ]}
      />
      <div className="import-head">
        <Checkbox checked={all} onChange={(on) => setChosen(new Set(on ? hosts.map((h) => h.id) : []))}>
          {t.hosts.import.hosts(hosts.length)}
        </Checkbox>
      </div>
      <div className="import-list export-list">
        {hosts.map((h) => {
          const p = h.proxy;
          const via = p?.type === "jump" ? ws.hosts.find((x) => x.id === p.hostId)?.label : undefined;
          return (
            <label key={h.id} className="checkbox import-row">
              <input type="checkbox" checked={chosen.has(h.id)} onChange={(e) => toggle(h.id, e.target.checked)} />
              <span className="checkbox-box"><Check size={12} strokeWidth={3} /></span>
              <span className="import-main">
                <span className="import-alias">{h.label}</span>
                <span className="import-addr mono">
                  {loginUser(ws, h)}@{h.address}{h.port !== 22 && `:${h.port}`}
                  {via && (
                    <>
                      {" "}
                      <ArrowRight size={11} /> {t.hosts.import.via(via)}
                    </>
                  )}
                </span>
              </span>
            </label>
          );
        })}
      </div>
      {format === "ssh" && keys.length > 0 && (
        <Checkbox checked={withKeys} onChange={setWithKeys}>
          {t.hosts.export.saveKeys(t.common.keyCount(keys.length))}
        </Checkbox>
      )}
      {format === "ssh" && text && <pre className="export-preview mono">{text}</pre>}
      {format === "file" && (
        <>
          {counts && INCLUDE.some((k) => counts[k] > 0) && (
            <div className="export-include">
              {INCLUDE.filter((k) => counts[k] > 0).map((k) => (
                <Checkbox key={k} checked={include[k]} onChange={(on) => setInclude((i) => ({ ...i, [k]: on }))}>
                  {countOf[k](counts[k])}
                </Checkbox>
              ))}
            </div>
          )}
          <div className="export-password">
            <Field label={t.hosts.export.filePassword} error={tried && !password ? passwordError : null}>
              <PasswordInput value={password} onChange={setPassword} invalid={tried && !password} autoFocus />
            </Field>
            <Field label={t.hosts.export.repeat} error={tried && password && repeat !== password ? passwordError : null}>
              <PasswordInput value={repeat} onChange={setRepeat} invalid={tried && !!password && repeat !== password} />
            </Field>
          </div>
        </>
      )}
    </Modal>
  );
}
