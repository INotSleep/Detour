// The terminal, its session lines, host key questions, and the login pieces of the host panel.
import type en from "../en/terminal";

const ru: typeof en = {
  offerSave: {
    password: (host: string) => `Сохранить пароль для «${host}»?`,
    passphrase: (key: string) => `Сохранить пароль ключа «${key}»?`,
    saved: "Сохранено в vault",
  },
  search: {
    placeholder: "Найти",
    count: (i: number, n: number) => `${i} из ${n}`,
    none: "Нет",
    prev: "Предыдущее · Shift+Enter",
    next: "Следующее · Enter",
    close: "Закрыть · Esc",
  },
  menu: {
    find: "Найти",
    splitRight: "Разделить вправо",
    splitDown: "Разделить вниз",
    splitRightWith: "Разделить вправо с…",
    splitDownWith: "Разделить вниз с…",
    toTab: "Вынести в новую вкладку",
  },
  hostKey: {
    label: "Ключ хоста",
    unknown: "Новый хост",
    changed: "Ключ хоста изменился",
    fingerprint: "Отпечаток",
    was: "Был",
    now: "Сейчас",
    replace: "Заменить ключ",
    stop: "Отменить",
    trust: "Доверять и подключиться",
  },
  session: {
    retry: "Enter: повторить",
    closed: "Соединение закрыто. Enter: переподключиться",
    cancelled: "Подключение отменено",
    cancelledKeyChanged: "Подключение отменено: ключ хоста изменился",
    passphrasePrompt: "Пароль ключа: ",
  },
  login: {
    passwordCopied: "Пароль скопирован",
    askOnConnect: "пароль при подключении",
    saved: "Свой вход сохранён",
    removed: "Свой вход убран",
    title: "Свой вход",
    useShared: "Входить общим",
    useOwn: "Входить со своими данными",
    usingShared: "Выключен: вход общий",
    switchedShared: "Вход общий",
    switchedOwn: "Вход со своими данными",
    setOwn: "Указать свой вход",
    user: "Пользователь",
    auth: "Вход",
    ownKey: "Свой ключ",
    noKeys: "Нет личных ключей",
    passwordSaved: "Сохранён",
    askPassword: "Спрашивать при подключении",
  },
};

export default ru;
