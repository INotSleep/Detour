// Demo data shown when the UI runs in a browser without the core.
const en = {
  you: "Vladislav",
  home: "Home",
  snippets: { restartApi: "Restart API", diskSpace: "Disk space", load: "Load", upgrade: "Upgrade system" },
  tunnels: { stagingProxy: "Proxy via staging", frontendDemo: "Frontend demo", ciMetrics: "CI metrics" },
  proxies: { office: "Office VPN", corporate: "Corporate proxy" },
  downloads: "Downloads",
  connectFailed: (endpoint: string) => `Couldn't connect: ${endpoint} is not responding`,
  enterRetry: "Enter: retry",
};

export default en;
