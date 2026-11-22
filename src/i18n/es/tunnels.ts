import { es as plural } from "../plural";
import type en from "../en/tunnels";

const es: typeof en = {
  title: "Túneles",
  needHost: "Un túnel necesita un host",
  toHosts: "Ir a hosts",
  createTunnel: "Crear túnel",
  empty: "No hay túneles",
  more: "Más",
  restore: "Deshacer",
  server: "Servidor",
  portTaken: (port: number, name: string) => `"${name}" ya usa el puerto ${port}`,
  removed: (name: string) => `"${name}" eliminado`,
  copied: (text: string) => `Copiado: ${text}`,
  close: "Cerrar túnel",
  open: "Abrir túnel",
  opening: "Abriendo",
  isOpen: "Abierto",
  connections: (n: number) => `${n} ${plural(n, { one: "conexión", other: "conexiones" })}`,
  route: {
    thisComputer: "Este equipo",
    port: (port: string | number) => `puerto ${port}`,
    anyAddress: "Cualquier dirección",
  },
  secret: {
    title: (name: string) => `"${name}"`,
    passphrase: "Frase de contraseña",
    passwordFor: (who: string) => `Contraseña de ${who}`,
    saveToVault: "Guardar en la bóveda",
  },
  trust: {
    title: "Host nuevo",
    trustAndOpen: "Confiar y abrir",
  },
  changed: {
    title: "La clave del host ha cambiado",
    replace: "Reemplazar clave",
    keep: "Cancelar",
    was: "Antes",
    now: "Ahora",
  },
  kinds: {
    local: "Abrir aquí un puerto del servidor",
    remote: "Exponer mi puerto en el servidor",
    dynamic: "Proxy a través del servidor",
  },
  dialog: {
    title: "Túnel",
    newTitle: "Nuevo túnel",
    server: "Servidor",
    serverFallback: "servidor",
    proxyVia: (via: string) => `Proxy vía ${via}`,
    onServer: (target: string, via: string) => `${target} en ${via}`,
    saved: "Guardado",
    created: (name: string) => `"${name}" creado`,
    port: "Puerto",
    address: "Dirección",
    anyTarget: "cualquier dirección : cualquier puerto",
    name: "Nombre",
    unreachable: "inaccesible",
  },
};

export default es;
