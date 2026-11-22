// The terminal, its session lines, host key questions, and the login pieces of the host panel.
const en = {
  offerSave: {
    password: (host: string) => `Save password for "${host}"?`,
    passphrase: (key: string) => `Save passphrase for key "${key}"?`,
    saved: "Saved to vault",
  },
  search: {
    placeholder: "Find",
    count: (i: number, n: number) => `${i} of ${n}`,
    none: "No results",
    prev: "Previous · Shift+Enter",
    next: "Next · Enter",
    close: "Close · Esc",
  },
  menu: {
    find: "Find",
    splitRight: "Split right",
    splitDown: "Split down",
    splitRightWith: "Split right with…",
    splitDownWith: "Split down with…",
    toTab: "Move to new tab",
  },
  hostKey: {
    label: "Host key",
    unknown: "New host",
    changed: "Host key changed",
    fingerprint: "Fingerprint",
    was: "Previous",
    now: "Current",
    replace: "Replace key",
    /** Backing out of a changed key: the safe choice. */
    stop: "Cancel",
    trust: "Trust and connect",
  },
  /** Lines the app writes into the terminal. */
  session: {
    retry: "Enter: retry",
    closed: "Connection closed. Enter: reconnect",
    cancelled: "Connection cancelled",
    cancelledKeyChanged: "Connection cancelled: host key changed",
    passphrasePrompt: "Key passphrase: ",
  },
  login: {
    passwordCopied: "Password copied",
    askOnConnect: "password on connect",
    saved: "Personal login saved",
    removed: "Personal login removed",
    title: "Personal login",
    useShared: "Use shared login",
    useOwn: "Use my own login",
    usingShared: "Off: the shared login is used",
    switchedShared: "Logging in with the shared login",
    switchedOwn: "Logging in with your own login",
    setOwn: "Set personal login",
    user: "Username",
    auth: "Authentication",
    ownKey: "My key",
    noKeys: "No personal keys",
    passwordSaved: "Saved",
    askPassword: "Ask when connecting",
  },
};

export default en;
