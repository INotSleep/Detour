import { de as plural } from "../plural";
import type en from "../en/keys";

const de: typeof en = {
  title: "Schlüssel",
  import: "Importieren",
  createKey: "Schlüssel erstellen",
  empty: "Keine Schlüssel",
  more: "Mehr",
  restore: "Rückgängig",
  passphraseSaved: "Passphrase gespeichert",
  withPassphrase: "Passphrase",
  copyPublic: "Öffentlichen Schlüssel kopieren",
  savePublic: "Öffentlichen Schlüssel speichern",
  exportPrivate: "Privaten Schlüssel exportieren",
  changePassphrase: "Passphrase ändern",
  setPassphrase: "Passphrase festlegen",
  publicCopied: (name: string) => `Öffentlicher Schlüssel von „${name}“ kopiert`,
  saved: (where: string) => `Gespeichert: ${where}`,
  removed: (name: string) => `Schlüssel „${name}“ gelöscht`,
  deleteConfirm: {
    title: (name: string) => `„${name}“ löschen?`,
    body: (n: number) =>
      `${n} ${plural(n, {
        one: "Host nutzt diesen Schlüssel und kann sich dann nicht mehr verbinden.",
        other: "Hosts nutzen diesen Schlüssel und können sich dann nicht mehr verbinden.",
      })}`,
  },
  passphrase: "Passphrase",
  savePassphrase: "Passphrase im Tresor speichern",
  noPassphrase: "Keine Passphrase",
  name: "Name",
  create: {
    title: "Neuer Schlüssel",
    type: "Typ",
    created: (name: string) => `Schlüssel „${name}“ erstellt`,
    copyPublic: "Öffentlichen Schlüssel kopieren",
  },
  importDialog: {
    title: "Schlüssel importieren",
    fileTooBig: "Die Datei ist zu groß für einen Schlüssel",
    imported: (name: string) => `Schlüssel „${name}“ importiert`,
    passphraseRequired: "Der Schlüssel hat eine Passphrase: gib sie ein",
    chooseFile: "Datei wählen",
    orDrop: "oder Datei hierher ziehen",
    detected: (format: string) => `${format}-Schlüssel`,
    passphrasePlaceholder: "Falls der Schlüssel eine hat",
  },
  export: {
    title: (name: string) => `„${name}“ exportieren`,
    saveFile: "Datei speichern",
    keepsPassphrase: "Die Datei wird mit der aktuellen Passphrase des Schlüssels geschützt.",
    filePassphrase: "Passphrase der Datei",
  },
  passphraseDialog: {
    title: (name: string) => `Passphrase für „${name}“`,
    removed: "Passphrase entfernt",
    changed: "Passphrase geändert",
    wrongCurrent: "Falsche aktuelle Passphrase",
    remove: "Passphrase entfernen",
    current: "Aktuelle Passphrase",
    next: "Neue Passphrase",
    emptyRemoves: "Leer lassen zum Entfernen",
    repeat: "Bestätigen",
    mismatch: "Passphrasen stimmen nicht überein",
  },
};

export default de;
