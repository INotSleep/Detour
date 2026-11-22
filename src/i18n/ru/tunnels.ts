import { ru as plural } from "../plural";
import type en from "../en/tunnels";

const ru: typeof en = {
  title: "Туннели",
  needHost: "Для туннеля нужен хост",
  toHosts: "К хостам",
  createTunnel: "Создать туннель",
  empty: "Нет туннелей",
  more: "Ещё",
  restore: "Вернуть",
  server: "Сервер",
  portTaken: (port: number, name: string) => `Порт ${port} занят «${name}»`,
  removed: (name: string) => `«${name}» удалён`,
  copied: (text: string) => `Скопировано: ${text}`,
  close: "Закрыть туннель",
  open: "Открыть туннель",
  opening: "Открывается",
  isOpen: "Открыт",
  connections: (n: number) => `${n} ${plural(n, { one: "подключение", few: "подключения", many: "подключений" })}`,
  route: {
    thisComputer: "Этот компьютер",
    port: (port: string | number) => `порт ${port}`,
    anyAddress: "любой адрес",
  },
  secret: {
    title: (name: string) => `«${name}»`,
    passphrase: "Пароль ключа",
    passwordFor: (who: string) => `Пароль ${who}`,
    saveToVault: "Сохранить в vault",
  },
  trust: {
    title: "Новый хост",
    trustAndOpen: "Доверять и открыть",
  },
  changed: {
    title: "Ключ хоста изменился",
    replace: "Заменить ключ",
    keep: "Отменить",
    was: "Был",
    now: "Сейчас",
  },
  kinds: {
    local: "Сервис сервера у себя",
    remote: "Свой сервис на сервере",
    dynamic: "Прокси через сервер",
  },
  dialog: {
    title: "Туннель",
    newTitle: "Новый туннель",
    server: "Сервер",
    serverFallback: "сервер",
    proxyVia: (via: string) => `Прокси через ${via}`,
    onServer: (target: string, via: string) => `${target} на ${via}`,
    saved: "Сохранено",
    created: (name: string) => `«${name}» создан`,
    port: "Порт",
    address: "Адрес",
    anyTarget: "любой адрес : любой порт",
    name: "Название",
    unreachable: "недоступен",
  },
};

export default ru;
