// The terminal, its session lines, host key questions, and the login pieces of the host panel.
import type en from "../en/terminal";

const pt: typeof en = {
  offerSave: {
    password: (host: string) => `Salvar a senha de "${host}"?`,
    passphrase: (key: string) => `Salvar a frase secreta da chave "${key}"?`,
    saved: "Salvo no cofre",
  },
  search: {
    placeholder: "Localizar",
    count: (i: number, n: number) => `${i} de ${n}`,
    none: "Nenhum resultado",
    prev: "Anterior · Shift+Enter",
    next: "Próximo · Enter",
    close: "Fechar · Esc",
  },
  menu: {
    find: "Localizar",
    splitRight: "Dividir à direita",
    splitDown: "Dividir abaixo",
    splitRightWith: "Dividir à direita com…",
    splitDownWith: "Dividir abaixo com…",
    toTab: "Mover para nova aba",
  },
  hostKey: {
    label: "Chave do host",
    unknown: "Novo host",
    changed: "A chave do host mudou",
    fingerprint: "Impressão digital",
    was: "Anterior",
    now: "Atual",
    replace: "Substituir chave",
    stop: "Cancelar",
    trust: "Confiar e conectar",
  },
  session: {
    retry: "Enter: tentar novamente",
    closed: "Conexão encerrada. Enter: reconectar",
    cancelled: "Conexão cancelada",
    cancelledKeyChanged: "Conexão cancelada: a chave do host mudou",
    passphrasePrompt: "Frase secreta da chave: ",
  },
  login: {
    passwordCopied: "Senha copiada",
    askOnConnect: "senha ao conectar",
    saved: "Login pessoal salvo",
    removed: "Login pessoal removido",
    title: "Login pessoal",
    useShared: "Usar login compartilhado",
    useOwn: "Usar meu próprio login",
    usingShared: "Desativado: é usado o login compartilhado",
    switchedShared: "Entrando com o login compartilhado",
    switchedOwn: "Entrando com seu próprio login",
    setOwn: "Definir login pessoal",
    user: "Nome de usuário",
    auth: "Autenticação",
    ownKey: "Minha chave",
    noKeys: "Nenhuma chave pessoal",
    passwordSaved: "Salva",
    askPassword: "Perguntar ao conectar",
  },
};

export default pt;
