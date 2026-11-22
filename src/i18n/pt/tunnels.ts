import { pt as plural } from "../plural";
import type en from "../en/tunnels";

const pt: typeof en = {
  title: "Túneis",
  needHost: "Um túnel precisa de um host",
  toHosts: "Ir para hosts",
  createTunnel: "Criar túnel",
  empty: "Nenhum túnel",
  more: "Mais",
  restore: "Desfazer",
  server: "Servidor",
  portTaken: (port: number, name: string) => `A porta ${port} está em uso por "${name}"`,
  removed: (name: string) => `"${name}" excluído`,
  copied: (text: string) => `Copiado: ${text}`,
  close: "Fechar túnel",
  open: "Abrir túnel",
  opening: "Abrindo",
  isOpen: "Aberto",
  connections: (n: number) => `${n} ${plural(n, { one: "conexão", other: "conexões" })}`,
  route: {
    thisComputer: "Este computador",
    port: (port: string | number) => `porta ${port}`,
    anyAddress: "Qualquer endereço",
  },
  secret: {
    title: (name: string) => `"${name}"`,
    passphrase: "Frase secreta",
    passwordFor: (who: string) => `Senha de ${who}`,
    saveToVault: "Salvar no cofre",
  },
  trust: {
    title: "Novo host",
    trustAndOpen: "Confiar e abrir",
  },
  changed: {
    title: "A chave do host mudou",
    replace: "Substituir chave",
    keep: "Cancelar",
    was: "Antes",
    now: "Agora",
  },
  kinds: {
    local: "Abrir uma porta do servidor aqui",
    remote: "Expor minha porta no servidor",
    dynamic: "Proxy pelo servidor",
  },
  dialog: {
    title: "Túnel",
    newTitle: "Novo túnel",
    server: "Servidor",
    serverFallback: "servidor",
    proxyVia: (via: string) => `Proxy via ${via}`,
    onServer: (target: string, via: string) => `${target} em ${via}`,
    saved: "Salvo",
    created: (name: string) => `"${name}" criado`,
    port: "Porta",
    address: "Endereço",
    anyTarget: "qualquer endereço : qualquer porta",
    name: "Nome",
    unreachable: "inacessível",
  },
};

export default pt;
