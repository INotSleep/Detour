import { useEffect, useState } from "react";
import { ChevronRight, Cloud, ExternalLink, Folder, FolderPlus, Link2, Loader2, Users } from "lucide-react";
import { api, driveNames, errorText, type DriveAccount, type DriveFolderRef, type DriveListing, type DriveProvider, type DriveStart } from "../lib/api";
import { copyText, useStore } from "../lib/store";
import { Button, Field, Modal, Segmented } from "./ui";
import { useT } from "../i18n";

/* ---------- signing in to a drive ---------- */

/** This device's sign-in to a drive: the account, or signing in (a code to enter, or a page to open). */
export function DriveSignIn({ provider, onAccount }: { provider: DriveProvider; onAccount: (account: string | null) => void }) {
  const t = useT();
  const d = t.teams.drive;
  const name = driveNames[provider];
  const { toast } = useStore();
  const [account, setAccount] = useState<DriveAccount | null | undefined>(undefined);
  const [start, setStart] = useState<DriveStart | null>(null);
  const [starting, setStarting] = useState(false);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setAccount(undefined);
    setStart(null);
    setError(null);
    api.driveAccounts().then((list) => setAccount(list.find((a) => a.provider === provider) ?? null)).catch(() => setAccount(null));
  }, [provider]);

  useEffect(() => onAccount(account?.signedIn ? account.account : null), [account]);

  // Waits for the sign-in to finish on the drive's page.
  useEffect(() => {
    if (!start || (start.kind === "browser" && start.paste)) return;
    let stop = false;
    let interval = start.kind === "device" ? start.interval : 2;
    let timer: ReturnType<typeof setTimeout>;
    const next = () => {
      timer = setTimeout(async () => {
        try {
          const r = await api.driveSignInPoll(provider);
          if (stop) return;
          if (r.status === "done") return done(r.account);
          if (r.status === "expired" || r.status === "denied") {
            setStart(null);
            return;
          }
          if (r.status === "slowDown") interval += 5;
          next();
        } catch (e) {
          if (stop) return;
          setStart(null);
          setError(errorText(e));
        }
      }, interval * 1000);
    };
    next();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [start]);

  const done = (who: string) => {
    setStart(null);
    setAccount({ provider, account: who, signedIn: true });
    toast({ text: d.signedIn(name, who), tone: "success" });
  };

  const begin = async () => {
    setStarting(true);
    setError(null);
    try {
      const s = await api.driveSignIn(provider);
      setStart(s);
      if (s.kind === "browser") await api.openInBrowser(s.url);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setStarting(false);
    }
  };

  const copyAndOpen = async (s: Extract<DriveStart, { kind: "device" }>) => {
    await copyText(s.userCode);
    try {
      await api.openInBrowser(s.verificationUri);
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const finishCode = async () => {
    if (!code.trim()) return;
    try {
      done(await api.driveSignInCode(provider, code.trim()));
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (account === undefined) return null;
  return (
    <div className="field">
      <span className="field-label">{name}</span>
      {start?.kind === "device" ? (
        <div className="github-code">
          <div className="input-group">
            <div className="id-box mono grow github-user-code">{start.userCode}</div>
            <Button type="button" variant="primary" icon={ExternalLink} onClick={() => void copyAndOpen(start)}>{d.copyOpen(name)}</Button>
          </div>
          <span className="muted small github-status"><Loader2 size={13} className="spin" />{d.waiting}</span>
        </div>
      ) : start?.kind === "browser" && start.paste ? (
        <div className="input-group">
          <input className="input mono grow" value={code} onChange={(e) => setCode(e.target.value)} placeholder={d.pasteCode} autoFocus />
          <Button type="button" variant="primary" disabled={!code.trim()} onClick={() => void finishCode()}>{t.common.done}</Button>
        </div>
      ) : start?.kind === "browser" ? (
        <div className="github-install">
          <span className="muted small github-status"><Loader2 size={13} className="spin" />{d.waiting}</span>
          <Button type="button" size="sm" icon={ExternalLink} onClick={() => void api.openInBrowser(start.url)}>{d.open(name)}</Button>
        </div>
      ) : account?.signedIn ? (
        <div className="github-account">
          <span className="mono small">{d.account(account.account)}</span>
        </div>
      ) : (
        <Button type="button" variant="primary" className="self-start" loading={starting} onClick={() => void begin()}>
          {account ? d.signInAgain(name) : d.signIn(name)}
        </Button>
      )}
      {error && <span className="field-error">{error}</span>}
    </div>
  );
}

/** Signing in to a drive again, from a banner: done once the account is there. */
export function DriveSignInDialog({ provider, onClose, onDone }: { provider: DriveProvider; onClose: () => void; onDone: () => void }) {
  const t = useT();
  return (
    <Modal title={t.teams.drive.signIn(driveNames[provider])} width={460} onClose={onClose} onSubmit={onClose} footer={<Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>}>
      <DriveSignIn provider={provider} onAccount={(a) => { if (a) onDone(); }} />
    </Modal>
  );
}

/* ---------- picking a folder ---------- */

const PROVIDERS: DriveProvider[] = ["onedrive", "gdrive", "dropbox"];

/** Which drive, the sign-in to it, and the vault's folder there (picked, or from a share link). */
export function DriveFields({ provider, folder, set }: { provider: DriveProvider; folder: DriveFolderRef | null; set: (p: { provider?: DriveProvider; drive?: DriveFolderRef | null }) => void }) {
  const t = useT();
  const d = t.teams.drive;
  const [account, setAccount] = useState<string | null>(null);
  const [browsing, setBrowsing] = useState(false);
  const [link, setLink] = useState("");
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const openLink = async () => {
    if (!link.trim() || opening) return;
    setOpening(true);
    setError(null);
    try {
      set({ drive: await api.driveLink(link.trim(), provider) });
      setLink("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setOpening(false);
    }
  };

  return (
    <>
      <Field label={d.service}>
        <Segmented value={provider} onChange={(p) => set({ provider: p, drive: null })} options={PROVIDERS.map((p) => ({ value: p, label: driveNames[p] }))} />
      </Field>
      <DriveSignIn provider={provider} onAccount={setAccount} />
      {account && (
        <>
          <Field label={d.folder}>
            <div className="input-group">
              <div className={`input mono grow drive-folder ${folder ? "" : "muted"}`}>{folder ? folder.label || driveNames[provider] : d.noFolder}</div>
              <Button type="button" icon={Folder} onClick={() => setBrowsing(true)}>{d.choose}</Button>
            </div>
          </Field>
          <Field label={d.link} error={error}>
            <div className="input-group">
              <input
                className="input mono grow"
                value={link}
                onChange={(e) => setLink(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void openLink();
                  }
                }}
                placeholder="https://…"
                spellCheck={false}
              />
              <Button type="button" icon={Link2} loading={opening} disabled={!link.trim()} onClick={() => void openLink()}>{d.openLink}</Button>
            </div>
          </Field>
        </>
      )}
      {browsing && <DriveBrowser provider={provider} onClose={() => setBrowsing(false)} onPick={(f) => { set({ drive: f }); setBrowsing(false); }} />}
    </>
  );
}

/** Walks the drive's folders, makes one if needed, and picks one. */
export function DriveBrowser({ provider, onClose, onPick }: { provider: DriveProvider; onClose: () => void; onPick: (f: DriveFolderRef) => void }) {
  const t = useT();
  const d = t.teams.drive;
  const name = driveNames[provider];
  // The folders walked into, from the top.
  const [trail, setTrail] = useState<DriveFolderRef[]>([]);
  const [listing, setListing] = useState<DriveListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [making, setMaking] = useState(false);
  const [newName, setNewName] = useState("");

  const here = trail[trail.length - 1] ?? null;
  const load = async (folder: DriveFolderRef | null) => {
    setListing(null);
    setError(null);
    try {
      setListing(await api.driveBrowse(provider, folder));
    } catch (e) {
      setError(errorText(e));
    }
  };
  useEffect(() => void load(here), [trail]);

  const enter = (f: DriveListing["folders"][number]) => {
    const parent = here?.label ?? "";
    const label = f.shared || !parent ? f.name : `${parent}/${f.name}`;
    setTrail([...trail, { provider, id: f.id, drive: f.drive ?? listing?.folder.drive ?? null, label }]);
  };

  const create = async () => {
    if (!newName.trim()) return;
    try {
      const f = await api.driveFolderCreate(provider, here, newName.trim());
      setMaking(false);
      setNewName("");
      setTrail([...trail, f]);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <Modal
      title={d.browseTitle(name)}
      width={520}
      onClose={onClose}
      onSubmit={() => here && onPick(here)}
      footer={
        <>
          <Button type="button" variant="ghost" className="footer-left" icon={FolderPlus} onClick={() => setMaking(true)}>{d.newFolder}</Button>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!here}>{d.useThis}</Button>
        </>
      }
    >
      <div className="drive-crumbs">
        <button type="button" className="crumb" onClick={() => setTrail([])}><Cloud size={13} />{name}</button>
        {trail.map((f, i) => (
          <span key={f.id} className="crumb-wrap">
            <ChevronRight size={13} className="muted" />
            <button type="button" className="crumb" onClick={() => setTrail(trail.slice(0, i + 1))}>{f.label.split("/").pop() || f.label}</button>
          </span>
        ))}
      </div>
      {making && (
        <div className="input-group">
          <input
            className="input grow"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void create();
              }
              if (e.key === "Escape") {
                e.stopPropagation();
                setMaking(false);
              }
            }}
            placeholder={d.folderName}
            autoFocus
          />
          <Button type="button" variant="primary" disabled={!newName.trim()} onClick={() => void create()}>{t.common.create}</Button>
        </div>
      )}
      <div className="drive-list">
        {!listing && !error && <div className="drive-empty muted"><Loader2 size={15} className="spin" /></div>}
        {listing && listing.folders.length === 0 && <div className="drive-empty muted small">{d.empty}</div>}
        {listing?.folders.map((f) => (
          <button type="button" key={`${f.drive ?? ""}/${f.id}`} className="drive-row" onClick={() => enter(f)}>
            {f.shared ? <Users size={15} /> : <Folder size={15} />}
            <span className="grow ellipsis">{f.name}</span>
            {f.shared && <span className="muted small">{d.shared}</span>}
            <ChevronRight size={14} className="muted" />
          </button>
        ))}
      </div>
      {error && <span className="field-error">{error}</span>}
    </Modal>
  );
}
