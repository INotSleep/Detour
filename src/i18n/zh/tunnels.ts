import { zh as plural } from "../plural";
import type en from "../en/tunnels";

const zh: typeof en = {
  title: "隧道",
  needHost: "隧道需要一个主机",
  toHosts: "前往主机",
  createTunnel: "创建隧道",
  empty: "没有隧道",
  more: "更多",
  restore: "撤销",
  server: "服务器",
  portTaken: (port: number, name: string) => `端口 ${port} 已被“${name}”占用`,
  removed: (name: string) => `已删除“${name}”`,
  copied: (text: string) => `已复制：${text}`,
  close: "关闭隧道",
  open: "打开隧道",
  opening: "正在打开",
  isOpen: "已打开",
  connections: (n: number) => `${n} ${plural(n, { other: "个连接" })}`,
  route: {
    thisComputer: "本机",
    port: (port: string | number) => `端口 ${port}`,
    anyAddress: "任意地址",
  },
  secret: {
    title: (name: string) => `“${name}”`,
    passphrase: "密码短语",
    passwordFor: (who: string) => `${who} 的密码`,
    saveToVault: "保存到保险库",
  },
  trust: {
    title: "新主机",
    trustAndOpen: "信任并打开",
  },
  changed: {
    title: "主机密钥已更改",
    replace: "替换密钥",
    keep: "取消",
    was: "之前",
    now: "现在",
  },
  kinds: {
    local: "在本机打开服务器端口",
    remote: "在服务器上开放我的端口",
    dynamic: "通过服务器代理",
  },
  dialog: {
    title: "隧道",
    newTitle: "新建隧道",
    server: "服务器",
    serverFallback: "服务器",
    proxyVia: (via: string) => `经由 ${via} 代理`,
    onServer: (target: string, via: string) => `${via} 上的 ${target}`,
    saved: "已保存",
    created: (name: string) => `已创建“${name}”`,
    port: "端口",
    address: "地址",
    anyTarget: "任意地址 : 任意端口",
    name: "名称",
    unreachable: "无法访问",
  },
};

export default zh;
