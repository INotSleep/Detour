// The terminal, its session lines, host key questions, and the login pieces of the host panel.
import type en from "../en/terminal";

const de: typeof en = {
  offerSave: {
    password: (host: string) => `Passwort für „${host}“ speichern?`,
    passphrase: (key: string) => `Passphrase für Schlüssel „${key}“ speichern?`,
    saved: "Im Tresor gespeichert",
  },
  search: {
    placeholder: "Suchen",
    count: (i: number, n: number) => `${i} von ${n}`,
    none: "Keine Treffer",
    prev: "Zurück · Shift+Enter",
    next: "Weiter · Enter",
    close: "Schließen · Esc",
  },
  menu: {
    find: "Suchen",
    splitRight: "Rechts teilen",
    splitDown: "Unten teilen",
    splitRightWith: "Rechts teilen mit…",
    splitDownWith: "Unten teilen mit…",
    toTab: "In neuen Tab verschieben",
  },
  hostKey: {
    label: "Host-Schlüssel",
    unknown: "Neuer Host",
    changed: "Host-Schlüssel geändert",
    fingerprint: "Fingerabdruck",
    was: "Vorher",
    now: "Aktuell",
    replace: "Schlüssel ersetzen",
    stop: "Abbrechen",
    trust: "Vertrauen und verbinden",
  },
  session: {
    retry: "Enter: erneut versuchen",
    closed: "Verbindung geschlossen. Enter: neu verbinden",
    cancelled: "Verbindung abgebrochen",
    cancelledKeyChanged: "Verbindung abgebrochen: Host-Schlüssel geändert",
    passphrasePrompt: "Passphrase: ",
  },
  login: {
    passwordCopied: "Passwort kopiert",
    askOnConnect: "Passwort beim Verbinden",
    saved: "Eigene Anmeldung gespeichert",
    removed: "Eigene Anmeldung entfernt",
    title: "Eigene Anmeldung",
    useShared: "Gemeinsame Anmeldung nutzen",
    useOwn: "Eigene Anmeldung nutzen",
    usingShared: "Aus: die gemeinsame Anmeldung wird genutzt",
    switchedShared: "Jetzt mit gemeinsamer Anmeldung",
    switchedOwn: "Jetzt mit eigener Anmeldung",
    setOwn: "Eigene Anmeldung festlegen",
    user: "Benutzername",
    auth: "Authentifizierung",
    ownKey: "Mein Schlüssel",
    noKeys: "Keine eigenen Schlüssel",
    passwordSaved: "Gespeichert",
    askPassword: "Beim Verbinden fragen",
  },
};

export default de;
