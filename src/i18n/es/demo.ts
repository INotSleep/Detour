// Demo data shown when the UI runs in a browser without the core.
import type en from "../en/demo";

const es: typeof en = {
  you: "Vladislav",
  home: "Casa",
  snippets: { restartApi: "Reiniciar API", diskSpace: "Espacio en disco", load: "Carga", upgrade: "Actualizar sistema" },
  tunnels: { stagingProxy: "Proxy vía staging", frontendDemo: "Demo del frontend", ciMetrics: "Métricas de CI" },
  proxies: { office: "VPN de la oficina", corporate: "Proxy corporativo" },
  downloads: "Descargas",
  connectFailed: (endpoint: string) => `No se pudo conectar: ${endpoint} no responde`,
  enterRetry: "Enter: reintentar",
};

export default es;
