import { useEffect, useState } from "react";
import { ArrowRight, Check, FileKey, FileWarning, Loader2 } from "lucide-react";
import { api, errorText, type FileCounts, type FileHost, type ImportHost, type ImportPreview } from "../lib/api";
import { uid, useStore } from "../lib/store";
import type { Host } from "../lib/types";
import { Button, Checkbox, Empty, Field, Modal, PasswordInput } from "./ui";
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

/** What a plain browser shows for a Detour file. */
const DEMO_FILE: { hosts: FileHost[]; counts: FileCounts } = {
  hosts: [
    { id: "f1", label: "gateway", address: "198.51.100.20", port: 22, user: "ops", group: "Office", via: null, exists: false },
    { id: "f2", label: "build-01", address: "10.20.0.11", port: 22, user: "ci", group: "Office", via: "gateway", exists: false },
    { id: "f3", label: "api-1", address: "10.0.1.11", port: 22, user: "deploy", group: "Production", via: null, exists: true },
  ],
  counts: { hosts: 3, passwords: 1, keys: 2, proxies: 0, knownHosts: 3, groups: 2 },
};

/** Hosts, keys and pinned server keys from ~/.ssh, into the personal vault; or a Detour file's. */
export function ImportDialog({ onClose }: { onClose: () => void }) {
  const [file, setFile] = useState<string | null>(null);
  if (file) return <FileImport path={file} onClose={onClose} />;
  const pick = () => void api.pickDetourFile().then((p) => p && setFile(p)).catch(() => {});
  return <SshImport onClose={onClose} onFile={pick} />;
}

function SshImport({ onClose, onFile }: { onClose: () => void; onFile: () => void }) {
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
          <Button type="button" variant="ghost" icon={FileKey} className="footer-left" onClick={onFile}>{t.hosts.import.fromFile}</Button>
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

/** A Detour file: opened with its password, then the chosen hosts come in with what logs in to them. */
function FileImport({ path, onClose }: { path: string; onClose: () => void }) {
  const t = useT();
  const { toast, refreshPersonal, update } = useStore();
  const [password, setPassword] = useState("");
  const [preview, setPreview] = useState<{ hosts: FileHost[]; counts: FileCounts } | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const openFile = async () => {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      const p = api.real ? await api.hostsFileOpen(path, password) : DEMO_FILE;
      setPreview(p);
      setChosen(new Set(p.hosts.filter((h) => !h.exists).map((h) => h.id)));
    } catch (e) {
      setError(errorText(e));
    }
    setBusy(false);
  };

  const importHosts = async () => {
    if (!preview || !chosen.size || busy) return;
    setBusy(true);
    try {
      if (api.real) {
        const r = await api.hostsFileImport(path, password, [...chosen]);
        await refreshPersonal();
        const parts = [t.common.hostCount(r.hosts)];
        if (r.keys) parts.push(t.common.keyCount(r.keys));
        if (r.proxies) parts.push(t.hosts.import.proxyCount(r.proxies));
        if (r.known) parts.push(t.hosts.import.serverKeyCount(r.known));
        toast({ text: t.hosts.import.imported(parts.join(", ")), tone: "success" });
      } else {
        const added: Host[] = preview.hosts
          .filter((h) => chosen.has(h.id))
          .map((h) => ({ id: uid("h"), vaultId: "personal", group: h.group, label: h.label, address: h.address, port: h.port, user: h.user, auth: { type: "agent" }, tags: [], hasPassword: false, os: "ubuntu", online: true }));
        update((w) => ({ ...w, hosts: [...w.hosts, ...added] }));
        toast({ text: t.hosts.import.imported(t.common.hostCount(added.length)), tone: "success" });
      }
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const toggle = (id: string, on: boolean) =>
    setChosen((s) => {
      const next = new Set(s);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  const name = path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
  return (
    <Modal
      title={t.hosts.import.fileTitle}
      width={600}
      onClose={onClose}
      onSubmit={preview ? importHosts : openFile}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          {preview ? (
            <Button type="submit" variant="primary" loading={busy} disabled={!chosen.size}>
              {chosen.size ? t.hosts.import.importHosts(chosen.size) : t.hosts.import.import}
            </Button>
          ) : (
            <Button type="submit" variant="primary" loading={busy} disabled={!password}>{t.hosts.import.open}</Button>
          )}
        </>
      }
    >
      {!preview && (
        <Field label={`${t.hosts.export.filePassword}: ${name}`} error={error}>
          <PasswordInput value={password} onChange={(v) => { setPassword(v); setError(null); }} invalid={!!error} autoFocus />
        </Field>
      )}
      {preview && (
        <>
          {error && <span className="field-error">{error}</span>}
          <div className="import-head">
            <Checkbox checked={chosen.size === preview.hosts.length} onChange={(on) => setChosen(new Set(on ? preview.hosts.map((h) => h.id) : []))}>
              {t.hosts.import.hosts(preview.hosts.length)}
            </Checkbox>
          </div>
          <div className="import-list">
            {preview.hosts.map((h) => (
              <label key={h.id} className="checkbox import-row">
                <input type="checkbox" checked={chosen.has(h.id)} onChange={(e) => toggle(h.id, e.target.checked)} />
                <span className="checkbox-box"><Check size={12} strokeWidth={3} /></span>
                <span className="import-main">
                  <span className="import-alias">{h.label}</span>
                  <span className="import-addr mono">
                    {h.user}@{h.address}{h.port !== 22 && `:${h.port}`}
                    {h.via && (
                      <>
                        {" "}
                        <ArrowRight size={11} /> {t.hosts.import.via(h.via)}
                      </>
                    )}
                  </span>
                </span>
                {h.exists && <span className="chip">{t.hosts.import.copy}</span>}
              </label>
            ))}
          </div>
        </>
      )}
    </Modal>
  );
}
