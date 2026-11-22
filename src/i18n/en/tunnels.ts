import { en as plural } from "../plural";

const en = {
  title: "Tunnels",
  needHost: "A tunnel needs a host",
  toHosts: "Go to hosts",
  createTunnel: "Create tunnel",
  empty: "No tunnels",
  more: "More",
  restore: "Undo",
  server: "Server",
  portTaken: (port: number, name: string) => `Port ${port} is used by "${name}"`,
  removed: (name: string) => `Deleted "${name}"`,
  copied: (text: string) => `Copied: ${text}`,
  close: "Close tunnel",
  open: "Open tunnel",
  opening: "Opening",
  isOpen: "Open",
  connections: (n: number) => `${n} ${plural(n, { one: "connection", other: "connections" })}`,
  route: {
    thisComputer: "This computer",
    port: (port: string | number) => `port ${port}`,
    anyAddress: "Any address",
  },
  secret: {
    title: (name: string) => `"${name}"`,
    passphrase: "Passphrase",
    passwordFor: (who: string) => `Password for ${who}`,
    saveToVault: "Save to vault",
  },
  trust: {
    title: "New host",
    trustAndOpen: "Trust and open",
  },
  changed: {
    title: "Host key changed",
    replace: "Replace key",
    keep: "Cancel",
    was: "Before",
    now: "Now",
  },
  kinds: {
    local: "Open a server port here",
    remote: "Expose my port on the server",
    dynamic: "Proxy through the server",
  },
  dialog: {
    title: "Tunnel",
    newTitle: "New tunnel",
    server: "Server",
    serverFallback: "server",
    proxyVia: (via: string) => `Proxy via ${via}`,
    onServer: (target: string, via: string) => `${target} on ${via}`,
    saved: "Saved",
    created: (name: string) => `Created "${name}"`,
    port: "Port",
    address: "Address",
    anyTarget: "any address : any port",
    name: "Name",
    unreachable: "unreachable",
  },
};

export default en;
