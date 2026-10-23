import { useEffect, useState } from "react";
import { ArrowRight, Check, FileWarning, Loader2 } from "lucide-react";
import { api, errorText, type ImportHost, type ImportPreview } from "../lib/api";
import { uid, useStore } from "../lib/store";
import type { Host } from "../lib/types";
import { Button, Checkbox, Empty, Modal } from "./ui";
import { useT } from "../i18n";

/** What a plain browser shows: a typical ~/.ssh/config. */
const DEMO: ImportPreview = {
  configPath: "~/.ssh/config",
  configFound: true,
  hosts: [
    { alias: "bastion", address: "203.0.113.10", port: 22, user: "ops", keyFile: "~/.ssh/id_ed25519", jumps: [], unsupported: null, exists: false, keyMissing: false },
    { alias: "db-internal", address: "10.10.0.7", port: 22, user: "postgres", keyFile: "~/.ssh/id_ed25519", jumps: ["bastion"], unsupported: null, exists: false, keyMissing: false },
    { alias: "old-box", address: "192.0.2.44", port: 2222, user: "root", keyFile: null, jumps: [], unsupported: "proxyCommand", exists: false, keyMissing: false },
    { alias: "api-1", address: "10.0.1.11", port: 22, user: "deploy", keyFile: null, jumps: [], unsupported: null, exists: true, keyMissing: false },
  ],
  knownNew: 12,
  knownDifferent: 0,
  knownHashed: 3,
};

/** Hosts, keys and pinned server keys from ~/.ssh, into the personal vault. */
export function ImportDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { update, toast, refreshPersonal } = useStore();
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [known, setKnown] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const load = api.real ? api.sshImportPreview() : Promise.resolve(DEMO);
    load
      .then((p) => {
        setPreview(p);
        setChosen(new Set(p.hosts.filter((h) => !h.exists).map((h) => h.alias)));
      })
      .catch((e) => setError(errorText(e)));
  }, []);

  const fresh = preview?.hosts.filter((h) => !h.exists) ?? [];
  const all = fresh.length > 0 && fresh.every((h) => chosen.has(h.alias));
  const toggle = (alias: string, on: boolean) =>
    setChosen((s) => {
      const next = new Set(s);
      if (on) next.add(alias);
      else next.delete(alias);
      return next;
    });
  const nothing = chosen.size === 0 && !(known && preview?.knownNew);

  const submit = async () => {
    if (!preview || nothing || busy) return;
    setBusy(true);
    try {
      let hosts = chosen.size;
      if (api.real) {
        const r = await api.sshImport([...chosen], known && preview.knownNew > 0);
        await refreshPersonal();
        hosts = r.hosts;
        const parts = [t.common.hostCount(r.hosts)];
        if (r.keys) parts.push(t.common.keyCount(r.keys));
        if (r.known) parts.push(t.hosts.import.serverKeyCount(r.known));
        toast({ text: t.hosts.import.imported(parts.join(", ")), tone: "success" });
      } else {
        const added: Host[] = preview.hosts
          .filter((h) => chosen.has(h.alias))
          .map((h) => ({ id: uid("h"), vaultId: "personal", group: "SSH config", label: h.alias, address: h.address, port: h.port, user: h.user, auth: { type: "agent" }, tags: [], hasPassword: false, os: "ubuntu", online: true }));
        update((w) => ({ ...w, hosts: [...w.hosts, ...added] }));
        toast({ text: t.hosts.import.imported(t.common.hostCount(hosts)), tone: "success" });
      }
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const count = chosen.size;
  return (
    <Modal
      title={t.hosts.importSsh}
      width={600}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!preview || nothing}>
            {count ? t.hosts.import.importHosts(count) : t.hosts.import.import}
          </Button>
        </>
      }
    >
      {!preview && !error && (
        <div className="import-wait">
          <Loader2 size={18} className="spin" />
        </div>
      )}
      {error && <span className="field-error">{error}</span>}
      {preview && !preview.configFound && preview.knownNew === 0 && <Empty icon={FileWarning} title={t.common.errors.notFound(preview.configPath)} />}
      {preview && preview.hosts.length > 0 && (
        <>
          <div className="import-head">
            <Checkbox checked={all} onChange={(on) => setChosen(new Set(on ? fresh.map((h) => h.alias) : []))}>
              {t.hosts.import.hosts(preview.hosts.length)}
            </Checkbox>
          </div>
          <div className="import-list">
            {preview.hosts.map((h) => (
              <ImportRow key={h.alias} h={h} checked={chosen.has(h.alias)} onChange={(on) => toggle(h.alias, on)} />
            ))}
          </div>
        </>
      )}
      {preview && preview.knownNew > 0 && (
        <Checkbox checked={known} onChange={setKnown}>
          {t.hosts.import.knownHosts(preview.knownNew)}
        </Checkbox>
      )}
    </Modal>
  );
}

function ImportRow({ h, checked, onChange }: { h: ImportHost; checked: boolean; onChange: (on: boolean) => void }) {
  const t = useT();
  const note = h.exists ? t.hosts.import.exists : h.unsupported ? t.hosts.import.proxyCommand : h.keyMissing ? t.hosts.import.keyMissing : null;
  return (
    <label className={`checkbox import-row ${h.exists ? "done" : ""}`}>
      <input type="checkbox" checked={checked && !h.exists} disabled={h.exists} onChange={(e) => onChange(e.target.checked)} />
      <span className="checkbox-box"><Check size={12} strokeWidth={3} /></span>
      <span className="import-main">
        <span className="import-alias">{h.alias}</span>
        <span className="import-addr mono">
          {h.user}@{h.address}{h.port !== 22 && `:${h.port}`}
          {h.jumps.length > 0 && (
            <>
              {" "}
              <ArrowRight size={11} /> {t.hosts.import.via(h.jumps.join(", "))}
            </>
          )}
        </span>
      </span>
      {note && <span className={`chip ${h.exists ? "" : "warn"}`}>{note}</span>}
    </label>
  );
}
