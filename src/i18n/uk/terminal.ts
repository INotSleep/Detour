// The terminal, its session lines, host key questions, and the login pieces of the host panel.
import type en from "../en/terminal";

const uk: typeof en = {
  offerSave: {
    password: (host: string) => `Зберегти пароль для «${host}»?`,
    passphrase: (key: string) => `Зберегти пароль ключа «${key}»?`,
    saved: "Збережено у сховищі",
  },
  search: {
    placeholder: "Знайти",
    count: (i: number, n: number) => `${i} з ${n}`,
    none: "Немає",
    prev: "Попереднє · Shift+Enter",
    next: "Наступне · Enter",
    close: "Закрити · Esc",
  },
  menu: {
    find: "Знайти",
    splitRight: "Розділити праворуч",
    splitDown: "Розділити донизу",
    splitRightWith: "Розділити праворуч з…",
    splitDownWith: "Розділити донизу з…",
    toTab: "Винести в нову вкладку",
  },
  hostKey: {
    label: "Ключ хоста",
    unknown: "Новий хост",
    changed: "Ключ хоста змінився",
    fingerprint: "Відбиток",
    was: "Був",
    now: "Зараз",
    replace: "Замінити ключ",
    stop: "Скасувати",
    trust: "Довіряти й підключитися",
  },
  session: {
    retry: "Enter: повторити",
    closed: "З'єднання закрито. Enter: перепідключитися",
    cancelled: "Підключення скасовано",
    cancelledKeyChanged: "Підключення скасовано: ключ хоста змінився",
    passphrasePrompt: "Пароль ключа: ",
  },
  login: {
    passwordCopied: "Пароль скопійовано",
    askOnConnect: "пароль під час підключення",
    saved: "Свій вхід збережено",
    removed: "Свій вхід прибрано",
    title: "Свій вхід",
    useShared: "Входити спільним",
    useOwn: "Входити зі своїми даними",
    usingShared: "Вимкнено: вхід спільний",
    switchedShared: "Вхід спільний",
    switchedOwn: "Вхід зі своїми даними",
    setOwn: "Указати свій вхід",
    user: "Користувач",
    auth: "Вхід",
    ownKey: "Свій ключ",
    noKeys: "Немає особистих ключів",
    passwordSaved: "Збережено",
    askPassword: "Питати під час підключення",
  },
};

export default uk;
