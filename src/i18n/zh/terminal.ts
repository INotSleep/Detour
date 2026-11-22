// The terminal, its session lines, host key questions, and the login pieces of the host panel.
import type en from "../en/terminal";

const zh: typeof en = {
  offerSave: {
    password: (host: string) => `保存“${host}”的密码？`,
    passphrase: (key: string) => `保存密钥“${key}”的密码短语？`,
    saved: "已保存到保险库",
  },
  search: {
    placeholder: "查找",
    count: (i: number, n: number) => `${i} / ${n}`,
    none: "无结果",
    prev: "上一个 · Shift+Enter",
    next: "下一个 · Enter",
    close: "关闭 · Esc",
  },
  menu: {
    find: "查找",
    splitRight: "向右拆分",
    splitDown: "向下拆分",
    splitRightWith: "向右拆分并打开…",
    splitDownWith: "向下拆分并打开…",
    toTab: "移到新标签页",
  },
  hostKey: {
    label: "主机密钥",
    unknown: "新主机",
    changed: "主机密钥已更改",
    fingerprint: "指纹",
    was: "之前",
    now: "当前",
    replace: "替换密钥",
    stop: "取消",
    trust: "信任并连接",
  },
  session: {
    retry: "Enter：重试",
    closed: "连接已关闭。Enter：重新连接",
    cancelled: "连接已取消",
    cancelledKeyChanged: "连接已取消：主机密钥已更改",
    passphrasePrompt: "密钥密码短语： ",
  },
  login: {
    passwordCopied: "密码已复制",
    askOnConnect: "连接时输入密码",
    saved: "个人登录信息已保存",
    removed: "个人登录信息已移除",
    title: "个人登录信息",
    useShared: "使用共享登录信息",
    useOwn: "使用我自己的登录信息",
    usingShared: "已关闭：使用共享登录信息",
    switchedShared: "将使用共享登录信息登录",
    switchedOwn: "将使用你自己的登录信息登录",
    setOwn: "设置个人登录信息",
    user: "用户名",
    auth: "认证方式",
    ownKey: "我的密钥",
    noKeys: "没有个人密钥",
    passwordSaved: "已保存",
    askPassword: "连接时询问",
  },
};

export default zh;
