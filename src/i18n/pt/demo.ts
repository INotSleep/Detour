// Demo data shown when the UI runs in a browser without the core.
import type en from "../en/demo";

const pt: typeof en = {
  you: "Vladislav",
  home: "Casa",
  snippets: { restartApi: "Reiniciar API", diskSpace: "Espaço em disco", load: "Carga", upgrade: "Atualizar sistema" },
  tunnels: { stagingProxy: "Proxy via staging", frontendDemo: "Demo do frontend", ciMetrics: "Métricas de CI" },
  proxies: { office: "VPN do escritório", corporate: "Proxy corporativo" },
  downloads: "Downloads",
  connectFailed: (endpoint: string) => `Não foi possível conectar: ${endpoint} não está respondendo`,
  enterRetry: "Enter: tentar novamente",
};

export default pt;
