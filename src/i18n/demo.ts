// Demo data shown when the UI runs in a browser without the core.

export const en = {
  you: "Vladislav",
  home: "Home",
  snippets: { restartApi: "Restart API", diskSpace: "Disk space", load: "Load", upgrade: "Upgrade system" },
  tunnels: { stagingProxy: "Proxy via staging", frontendDemo: "Frontend demo", ciMetrics: "CI metrics" },
  downloads: "Downloads",
  connectFailed: (endpoint: string) => `Couldn't connect: ${endpoint} is not responding`,
  enterRetry: "Enter: retry",
};

export const ru: typeof en = {
  you: "Владислав",
  home: "Дом",
  snippets: { restartApi: "Перезапуск API", diskSpace: "Место на дисках", load: "Нагрузка", upgrade: "Обновить систему" },
  tunnels: { stagingProxy: "Прокси через staging", frontendDemo: "Демо фронтенда", ciMetrics: "Метрики CI" },
  downloads: "Загрузки",
  connectFailed: (endpoint: string) => `Не удалось подключиться: ${endpoint} не отвечает`,
  enterRetry: "Enter: повторить",
};
