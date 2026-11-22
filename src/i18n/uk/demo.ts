// Demo data shown when the UI runs in a browser without the core.
import type en from "../en/demo";

const uk: typeof en = {
  you: "Владислав",
  home: "Дім",
  snippets: { restartApi: "Перезапуск API", diskSpace: "Місце на дисках", load: "Навантаження", upgrade: "Оновити систему" },
  tunnels: { stagingProxy: "Проксі через staging", frontendDemo: "Демо фронтенду", ciMetrics: "Метрики CI" },
  proxies: { office: "VPN офісу", corporate: "Корпоративний проксі" },
  downloads: "Завантаження",
  connectFailed: (endpoint: string) => `Не вдалося підключитися: ${endpoint} не відповідає`,
  enterRetry: "Enter: повторити",
};

export default uk;
