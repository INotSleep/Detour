import { uk as plural } from "../plural";
import type en from "../en/tunnels";

const uk: typeof en = {
  title: "Тунелі",
  needHost: "Для тунелю потрібен хост",
  toHosts: "До хостів",
  createTunnel: "Створити тунель",
  empty: "Немає тунелів",
  more: "Ще",
  restore: "Повернути",
  server: "Сервер",
  portTaken: (port: number, name: string) => `Порт ${port} зайнятий «${name}»`,
  removed: (name: string) => `Видалено «${name}»`,
  copied: (text: string) => `Скопійовано: ${text}`,
  close: "Закрити тунель",
  open: "Відкрити тунель",
  opening: "Відкривається",
  isOpen: "Відкрито",
  connections: (n: number) => `${n} ${plural(n, { one: "підключення", few: "підключення", many: "підключень" })}`,
  route: {
    thisComputer: "Цей комп'ютер",
    port: (port: string | number) => `порт ${port}`,
    anyAddress: "будь-яка адреса",
  },
  secret: {
    title: (name: string) => `«${name}»`,
    passphrase: "Пароль ключа",
    passwordFor: (who: string) => `Пароль для ${who}`,
    saveToVault: "Зберегти у сховищі",
  },
  trust: {
    title: "Новий хост",
    trustAndOpen: "Довіряти й відкрити",
  },
  changed: {
    title: "Ключ хоста змінився",
    replace: "Замінити ключ",
    keep: "Скасувати",
    was: "Був",
    now: "Зараз",
  },
  kinds: {
    local: "Сервіс сервера в себе",
    remote: "Свій сервіс на сервері",
    dynamic: "Проксі через сервер",
  },
  dialog: {
    title: "Тунель",
    newTitle: "Новий тунель",
    server: "Сервер",
    serverFallback: "сервер",
    proxyVia: (via: string) => `Проксі через ${via}`,
    onServer: (target: string, via: string) => `${target} на ${via}`,
    saved: "Збережено",
    created: (name: string) => `Створено «${name}»`,
    port: "Порт",
    address: "Адреса",
    anyTarget: "будь-яка адреса : будь-який порт",
    name: "Назва",
    unreachable: "недоступний",
  },
};

export default uk;
