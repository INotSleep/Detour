import { de as plural } from "../plural";
import type en from "../en/tunnels";

const de: typeof en = {
  title: "Tunnel",
  needHost: "Ein Tunnel braucht einen Host",
  toHosts: "Zu den Hosts",
  createTunnel: "Tunnel erstellen",
  empty: "Keine Tunnel",
  more: "Mehr",
  restore: "Rückgängig",
  server: "Server",
  portTaken: (port: number, name: string) => `Port ${port} wird von „${name}“ genutzt`,
  removed: (name: string) => `„${name}“ gelöscht`,
  copied: (text: string) => `Kopiert: ${text}`,
  close: "Tunnel schließen",
  open: "Tunnel öffnen",
  opening: "Wird geöffnet",
  isOpen: "Offen",
  connections: (n: number) => `${n} ${plural(n, { one: "Verbindung", other: "Verbindungen" })}`,
  route: {
    thisComputer: "Dieser Computer",
    port: (port: string | number) => `Port ${port}`,
    anyAddress: "Beliebige Adresse",
  },
  secret: {
    title: (name: string) => `„${name}“`,
    passphrase: "Passphrase",
    passwordFor: (who: string) => `Passwort für ${who}`,
    saveToVault: "Im Tresor speichern",
  },
  trust: {
    title: "Neuer Host",
    trustAndOpen: "Vertrauen und öffnen",
  },
  changed: {
    title: "Host-Schlüssel geändert",
    replace: "Schlüssel ersetzen",
    keep: "Abbrechen",
    was: "Vorher",
    now: "Jetzt",
  },
  kinds: {
    local: "Server-Port hier öffnen",
    remote: "Meinen Port auf dem Server freigeben",
    dynamic: "Proxy über den Server",
  },
  dialog: {
    title: "Tunnel",
    newTitle: "Neuer Tunnel",
    server: "Server",
    serverFallback: "Server",
    proxyVia: (via: string) => `Proxy über ${via}`,
    onServer: (target: string, via: string) => `${target} auf ${via}`,
    saved: "Gespeichert",
    created: (name: string) => `„${name}“ erstellt`,
    port: "Port",
    address: "Adresse",
    anyTarget: "beliebige Adresse : beliebiger Port",
    name: "Name",
    unreachable: "nicht erreichbar",
  },
};

export default de;
