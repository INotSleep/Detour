// Demo data shown when the UI runs in a browser without the core.
import type en from "../en/demo";

const de: typeof en = {
  you: "Vladislav",
  home: "Zuhause",
  snippets: { restartApi: "API neu starten", diskSpace: "Speicherplatz", load: "Auslastung", upgrade: "System aktualisieren" },
  tunnels: { stagingProxy: "Proxy über Staging", frontendDemo: "Frontend-Demo", ciMetrics: "CI-Metriken" },
  proxies: { office: "Büro-VPN", corporate: "Firmen-Proxy" },
  downloads: "Downloads",
  connectFailed: (endpoint: string) => `Verbindung fehlgeschlagen: ${endpoint} antwortet nicht`,
  enterRetry: "Enter: erneut versuchen",
};

export default de;
