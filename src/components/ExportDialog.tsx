import { useMemo, useState } from "react";
import { ArrowRight, Check, Copy, Save } from "lucide-react";
import { api, errorText } from "../lib/api";
import { copyText, useStore } from "../lib/store";
import { keyFileName, keysOf, sshConfig, withJumps } from "../lib/sshConfig";
import type { Host } from "../lib/types";
import { Button, Checkbox, Modal } from "./ui";
import { useT } from "../i18n";

/** Hosts as ~/.ssh/config entries, copied or saved to a file, with their keys if asked. */
export function ExportDialog({ hosts, initial, onClose }: { hosts: Host[]; initial?: string[]; onClose: () => void }) {
  const t = useT();
  const { ws, toast } = useStore();
  const [chosen, setChosen] = useState<Set<string>>(new Set(initial ?? hosts.map((h) => h.id)));
  const [withKeys, setWithKeys] = useState(false);
  const [busy, setBusy] = useState(false);

  const picked = hosts.filter((h) => chosen.has(h.id));
  const keys = useMemo(() => keysOf(withJumps(picked, ws.hosts), ws.keys), [picked, ws.hosts, ws.keys]);
  const text = useMemo(() => (picked.length ? sshConfig(picked, ws.hosts, ws.keys) : ""), [picked, ws.hosts, ws.keys]);
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
      const paths: Record<string, string> = {};
      if (saveKeys) {
        const where = await api.keyFilesSave(dir, keys.map((key) => ({ key, name: keyFileName(key.name) })));
        keys.forEach((k, i) => (paths[k.id] = where[i]));
      }
      await api.writeTextFile(path, sshConfig(picked, ws.hosts, ws.keys, paths));
      toast({ text: saveKeys ? t.hosts.export.savedWithKeys(path, t.common.keyCount(keys.length)) : t.hosts.export.saved(path), tone: "success" });
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
      onSubmit={save}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          {!saveKeys && <Button type="button" icon={Copy} disabled={!picked.length} onClick={copy}>{t.hosts.export.copy}</Button>}
          <Button type="submit" variant="primary" icon={Save} loading={busy} disabled={!picked.length}>{t.hosts.export.save}</Button>
        </>
      }
    >
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
                  {h.own?.user ?? h.user}@{h.address}{h.port !== 22 && `:${h.port}`}
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
      {keys.length > 0 && (
        <Checkbox checked={withKeys} onChange={setWithKeys}>
          {t.hosts.export.saveKeys(t.common.keyCount(keys.length))}
        </Checkbox>
      )}
      {text && <pre className="export-preview mono">{text}</pre>}
    </Modal>
  );
}
