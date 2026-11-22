// Demo data shown when the UI runs in a browser without the core.
import type en from "../en/demo";

const zh: typeof en = {
  you: "李明",
  home: "家庭",
  snippets: { restartApi: "重启 API", diskSpace: "磁盘空间", load: "负载", upgrade: "升级系统" },
  tunnels: { stagingProxy: "经由 staging 代理", frontendDemo: "前端演示", ciMetrics: "CI 指标" },
  proxies: { office: "办公室 VPN", corporate: "公司代理" },
  downloads: "下载",
  connectFailed: (endpoint: string) => `无法连接：${endpoint} 无响应`,
  enterRetry: "Enter：重试",
};

export default zh;
