// The terminal, its session lines, host key questions, and the login pieces of the host panel.
import type en from "../en/terminal";

const es: typeof en = {
  offerSave: {
    password: (host: string) => `¿Guardar la contraseña de "${host}"?`,
    passphrase: (key: string) => `¿Guardar la frase de contraseña de la clave "${key}"?`,
    saved: "Guardado en la bóveda",
  },
  search: {
    placeholder: "Buscar",
    count: (i: number, n: number) => `${i} de ${n}`,
    none: "Sin resultados",
    prev: "Anterior · Shift+Enter",
    next: "Siguiente · Enter",
    close: "Cerrar · Esc",
  },
  menu: {
    find: "Buscar",
    splitRight: "Dividir a la derecha",
    splitDown: "Dividir hacia abajo",
    splitRightWith: "Dividir a la derecha con…",
    splitDownWith: "Dividir hacia abajo con…",
    toTab: "Mover a una pestaña nueva",
  },
  hostKey: {
    label: "Clave del host",
    unknown: "Host nuevo",
    changed: "La clave del host ha cambiado",
    fingerprint: "Huella",
    was: "Anterior",
    now: "Actual",
    replace: "Reemplazar clave",
    stop: "Cancelar",
    trust: "Confiar y conectar",
  },
  session: {
    retry: "Enter: reintentar",
    closed: "Conexión cerrada. Enter: volver a conectar",
    cancelled: "Conexión cancelada",
    cancelledKeyChanged: "Conexión cancelada: la clave del host ha cambiado",
    passphrasePrompt: "Frase de contraseña de la clave: ",
  },
  login: {
    passwordCopied: "Contraseña copiada",
    askOnConnect: "contraseña al conectar",
    saved: "Inicio de sesión personal guardado",
    removed: "Inicio de sesión personal quitado",
    title: "Inicio de sesión personal",
    useShared: "Usar el inicio de sesión compartido",
    useOwn: "Usar mi propio inicio de sesión",
    usingShared: "Desactivado: se usa el inicio de sesión compartido",
    switchedShared: "Se inicia sesión con el compartido",
    switchedOwn: "Se inicia sesión con el tuyo propio",
    setOwn: "Definir inicio de sesión personal",
    user: "Nombre de usuario",
    auth: "Autenticación",
    ownKey: "Mi clave",
    noKeys: "No hay claves personales",
    passwordSaved: "Guardada",
    askPassword: "Preguntar al conectar",
  },
};

export default es;
