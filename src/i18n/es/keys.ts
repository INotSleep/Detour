import { es as plural } from "../plural";
import type en from "../en/keys";

const es: typeof en = {
  title: "Claves",
  import: "Importar",
  createKey: "Crear clave",
  empty: "No hay claves",
  more: "Más",
  restore: "Deshacer",
  passphraseSaved: "Frase de contraseña guardada",
  withPassphrase: "Frase de contraseña",
  copyPublic: "Copiar clave pública",
  savePublic: "Guardar clave pública",
  exportPrivate: "Exportar clave privada",
  changePassphrase: "Cambiar frase de contraseña",
  setPassphrase: "Establecer frase de contraseña",
  publicCopied: (name: string) => `Clave pública de "${name}" copiada`,
  saved: (where: string) => `Guardado: ${where}`,
  removed: (name: string) => `Clave "${name}" eliminada`,
  deleteConfirm: {
    title: (name: string) => `¿Eliminar "${name}"?`,
    body: (n: number) =>
      `${n} ${plural(n, { one: "host usa", other: "hosts usan" })} esta clave y ${plural(n, { one: "dejará", other: "dejarán" })} de conectarse.`,
  },
  passphrase: "Frase de contraseña",
  savePassphrase: "Guardar frase de contraseña en la bóveda",
  noPassphrase: "Sin frase de contraseña",
  name: "Nombre",
  create: {
    title: "Nueva clave",
    type: "Tipo",
    created: (name: string) => `Clave "${name}" creada`,
    copyPublic: "Copiar clave pública",
  },
  importDialog: {
    title: "Importar clave",
    fileTooBig: "El archivo es demasiado grande para ser una clave",
    imported: (name: string) => `Clave "${name}" importada`,
    passphraseRequired: "La clave tiene frase de contraseña: escríbela",
    chooseFile: "Elegir archivo",
    orDrop: "o suelta un archivo aquí",
    detected: (format: string) => `Clave ${format}`,
    passphrasePlaceholder: "Si la clave tiene una",
  },
  export: {
    title: (name: string) => `Exportar "${name}"`,
    saveFile: "Guardar archivo",
    keepsPassphrase: "El archivo quedará protegido con la frase de contraseña actual de la clave.",
    filePassphrase: "Frase de contraseña del archivo",
  },
  passphraseDialog: {
    title: (name: string) => `Frase de contraseña de "${name}"`,
    removed: "Frase de contraseña quitada",
    changed: "Frase de contraseña cambiada",
    wrongCurrent: "La frase de contraseña actual es incorrecta",
    remove: "Quitar frase de contraseña",
    current: "Frase de contraseña actual",
    next: "Nueva frase de contraseña",
    emptyRemoves: "Déjala vacía para quitarla",
    repeat: "Confirmar",
    mismatch: "Las frases de contraseña no coinciden",
  },
};

export default es;
