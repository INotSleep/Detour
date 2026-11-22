import { useEffect, useState } from "react";
import { ArrowRight, Check, FileWarning, FolderOpen, Loader2 } from "lucide-react";
import { api, errorText, isCoreError, type FileCounts, type FileHost, type ForeignClient, type ForeignHost, type ImportHost, type ImportPreview } from "../lib/api";
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

type Source = "ssh" | "putty" | "kitty" | "mobaxterm" | "securecrt" | "termius" | "csv" | "detour";
const SOURCES: Source[] = ["ssh", "putty", "kitty", "mobaxterm", "securecrt", "termius", "csv", "detour"];
type Foreign = Exclude<Source, "ssh" | "detour">;

/** Hosts into the personal vault: from ~/.ssh, another SSH client, a list, or a Detour file. */
export function ImportDialog({ onClose }: { onClose: () => void }) {
  const t = useT().hosts.import;
  const [source, setSource] = useState<Source>("ssh");
  const [file, setFile] = useState<string | null>(null);
  if (file) return <FileImport path={file} onClose={onClose} />;
  const choose = (s: Source) => {
    if (s === "detour") void api.pickDetourFile().then((p) => p && setFile(p)).catch(() => {});
    else setSource(s);
  };
  const picker = (
    <div className="import-source">
      <span>{t.from}</span>
      <select className="input input-sm" value={source} onChange={(e) => choose(e.target.value as Source)}>
        {SOURCES.map((s) => <option key={s} value={s}>{t.sources[s]}</option>)}
      </select>
    </div>
  );
  return source === "ssh" ? <SshImport onClose={onClose} picker={picker} /> : <ForeignImport key={source} source={source as Foreign} picker={picker} onClose={onClose} />;
}

function SshImport({ onClose, picker }: { onClose: () => void; picker: React.ReactNode }) {
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
      {picker}
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

/** What a plain browser shows for another client. */
const DEMO_FOREIGN: ForeignHost[] = [
  { key: "web", label: "web-prod", address: "203.0.113.21", port: 22, user: "deploy", group: "Production", keyFile: "C:\\keys\\web.ppk", hasPassword: false, jump: null, tunnels: [{ kind: "local", bindPort: 8080, target: "localhost:80" }], exists: false, keyMissing: false },
  { key: "db", label: "db-prod", address: "10.20.0.5", port: 22, user: "postgres", group: "Production", keyFile: null, hasPassword: false, jump: { key: null, address: "203.0.113.21", port: 22, user: "deploy" }, tunnels: [], exists: false, keyMissing: false },
  { key: "nas", label: "nas", address: "192.168.1.20", port: 2222, user: "admin", group: "", keyFile: "C:\\keys\\old.ppk", hasPassword: true, jump: null, tunnels: [], exists: false, keyMissing: true },
  { key: "api", label: "api-1", address: "10.0.1.11", port: 22, user: "deploy", group: "", keyFile: null, hasPassword: false, jump: null, tunnels: [], exists: true, keyMissing: false },
];

/** Where each client keeps its hosts, and what can be pointed at instead. */
const FOREIGN: Record<Foreign, { client: ForeignClient | null; name: string; finds: boolean; folder: boolean; ext: string[] }> = {
  putty: { client: "putty", name: "PuTTY", finds: true, folder: true, ext: ["reg"] },
  kitty: { client: "kitty", name: "KiTTY", finds: true, folder: true, ext: ["reg"] },
  mobaxterm: { client: "mobaxterm", name: "MobaXterm", finds: true, folder: false, ext: ["ini", "mxtsessions"] },
  securecrt: { client: "securecrt", name: "SecureCRT", finds: true, folder: true, ext: ["xml"] },
  // Termius gives a CSV or an SSH config: which one, the file's name says.
  termius: { client: null, name: "Termius", finds: false, folder: false, ext: ["csv", "txt", "conf", "config"] },
  csv: { client: "csv", name: "CSV", finds: false, folder: false, ext: ["csv", "txt"] },
};

function ForeignImport({ source, picker, onClose }: { source: Foreign; picker: React.ReactNode; onClose: () => void }) {
  const t = useT();
  const ti = t.hosts.import;
  const { update, toast, refreshPersonal } = useStore();
  const spec = FOREIGN[source];
  const [path, setPath] = useState<string | null>(null);
  const [found, setFound] = useState<{ source: string; hosts: ForeignHost[] } | null>(null);
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  const clientFor = (p: string | null): ForeignClient => spec.client ?? (p && /\.(csv|txt)$/i.test(p) ? "csv" : "sshConfig");

  const load = async (p: string | null) => {
    setLoading(true);
    setError(null);
    setMissing(false);
    try {
      const f = api.real ? await api.foreignPreview(clientFor(p), p) : { source: p ?? spec.name, hosts: DEMO_FOREIGN };
      setPath(p);
      setFound(f);
      setChosen(new Set(f.hosts.filter((h) => !h.exists).map((h) => h.key)));
    } catch (e) {
      if (isCoreError(e) && e.kind === "notFound" && !p) setMissing(true);
      else setError(errorText(e));
    }
    setLoading(false);
  };
  const pick = async (folder: boolean) => {
    const p = await api.pickImportSource(folder, spec.ext).catch(() => null);
    if (p) await load(p);
    else if (!found && !spec.finds) setMissing(true);
  };

  useEffect(() => {
    if (spec.finds) void load(null);
    else void pick(false);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const hosts = found?.hosts ?? [];
  const fresh = hosts.filter((h) => !h.exists);
  const toggle = (key: string, on: boolean) =>
    setChosen((s) => {
      const next = new Set(s);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });

  const submit = async () => {
    if (!chosen.size || busy) return;
    setBusy(true);
    try {
      if (api.real) {
        const r = await api.foreignImport(clientFor(path), path, [...chosen]);
        await refreshPersonal();
        const parts = [t.common.hostCount(r.hosts)];
        if (r.keys) parts.push(t.common.keyCount(r.keys));
        if (r.tunnels) parts.push(ti.tunnelCount(r.tunnels));
        toast({ text: ti.imported(parts.join(", ")), tone: "success" });
      } else {
        const added: Host[] = hosts
          .filter((h) => chosen.has(h.key))
          .map((h) => ({ id: uid("h"), vaultId: "personal", group: h.group || spec.name, label: h.label, address: h.address, port: h.port, user: h.user, auth: { type: "agent" }, tags: [], hasPassword: h.hasPassword, os: "ubuntu", online: true }));
        update((w) => ({ ...w, hosts: [...w.hosts, ...added] }));
        toast({ text: ti.imported(t.common.hostCount(added.length)), tone: "success" });
      }
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const pickButtons = (
    <>
      <Button type="button" size="sm" variant="ghost" icon={FolderOpen} onClick={() => void pick(false)}>{found || missing ? ti.otherFile : ti.pickFile}</Button>
      {spec.folder && <Button type="button" size="sm" variant="ghost" onClick={() => void pick(true)}>{ti.pickFolder}</Button>}
    </>
  );

  return (
    <Modal
      title={t.hosts.importSsh}
      width={600}
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!chosen.size}>
            {chosen.size ? ti.importHosts(chosen.size) : ti.import}
          </Button>
        </>
      }
    >
      {picker}
      {(found || missing) && (
        <div className="import-from">
          <span className="muted mono small ellipsis grow selectable">{found?.source ?? ""}</span>
          {pickButtons}
        </div>
      )}
      {loading && (
        <div className="import-wait">
          <Loader2 size={18} className="spin" />
        </div>
      )}
      {error && <span className="field-error">{error}</span>}
      {!loading && missing && !found && <Empty icon={FileWarning} title={spec.finds ? ti.notFound(spec.name) : ti.pickFile} />}
      {!loading && found && hosts.length === 0 && <Empty icon={FileWarning} title={ti.nothing(found.source)} />}
      {!loading && hosts.length > 0 && (
        <>
          <div className="import-head">
            <Checkbox checked={fresh.length > 0 && fresh.every((h) => chosen.has(h.key))} onChange={(on) => setChosen(new Set(on ? fresh.map((h) => h.key) : []))}>
              {ti.hosts(hosts.length)}
            </Checkbox>
          </div>
          <div className="import-list">
            {hosts.map((h) => {
              const note = h.exists ? ti.exists : h.keyMissing ? ti.keyMissing : null;
              return (
                <label key={h.key} className={`checkbox import-row ${h.exists ? "done" : ""}`}>
                  <input type="checkbox" checked={chosen.has(h.key) && !h.exists} disabled={h.exists} onChange={(e) => toggle(h.key, e.target.checked)} />
                  <span className="checkbox-box"><Check size={12} strokeWidth={3} /></span>
                  <span className="import-main">
                    <span className="import-alias">
                      {h.group && <span className="muted">{h.group} / </span>}
                      {h.label}
                    </span>
                    <span className="import-addr mono">
                      {h.user && `${h.user}@`}{h.address}{h.port !== 22 && `:${h.port}`}
                      {h.jump && (
                        <>
                          {" "}
                          <ArrowRight size={11} /> {ti.via(h.jump.address)}
                        </>
                      )}
                    </span>
                  </span>
                  {h.tunnels.length > 0 && !h.exists && <span className="chip">{ti.tunnelCount(h.tunnels.length)}</span>}
                  {h.hasPassword && !h.exists && <span className="chip">{ti.password}</span>}
                  {note && <span className={`chip ${h.exists ? "" : "warn"}`}>{note}</span>}
                </label>
              );
            })}
          </div>
        </>
      )}
    </Modal>
  );
}
