// Demo data shown when the UI runs in a browser without the core.
import type en from "../en/demo";

const ru: typeof en = {
  you: "Владислав",
  home: "Дом",
  snippets: { restartApi: "Перезапуск API", diskSpace: "Место на дисках", load: "Нагрузка", upgrade: "Обновить систему" },
  tunnels: { stagingProxy: "Прокси через staging", frontendDemo: "Демо фронтенда", ciMetrics: "Метрики CI" },
  proxies: { office: "VPN офиса", corporate: "Корпоративный прокси" },
  downloads: "Загрузки",
  connectFailed: (endpoint: string) => `Не удалось подключиться: ${endpoint} не отвечает`,
  enterRetry: "Enter: повторить",
};

export default ru;
