// Updates from the project's GitHub releases (Tauri's updater: signed, checked against the key built into the app).
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface UpdateInfo {
  version: string;
  notes?: string;
}

/** The update found by the last check, to install. */
let found: Update | null = null;

/** Nothing to compare with yet: not a failure. */
export const NO_RELEASES = "Нет опубликованных версий";

/** Why a check failed, in words. */
export function updateErrorText(e: unknown): string {
  const text = String(e instanceof Error ? e.message : e);
  if (/release JSON|404|not found/i.test(text)) return NO_RELEASES;
  if (/signature/i.test(text)) return "Обновление не прошло проверку подписи";
  if (/network|dns|connect|timed? ?out|error sending request/i.test(text)) return "Нет связи с сервером обновлений";
  return text;
}

/** The newest release if it is newer than this app; null if this is the newest. */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  found = await check();
  return found ? { version: found.version, notes: found.body } : null;
}

/** Downloads and installs the update found, reporting how much came in (0..1), then restarts the app. */
export async function installUpdate(onProgress: (share: number | null) => void): Promise<void> {
  if (!found) throw new Error("no update");
  let total = 0;
  let done = 0;
  await found.downloadAndInstall((e) => {
    if (e.event === "Started") total = e.data.contentLength ?? 0;
    if (e.event === "Progress") {
      done += e.data.chunkLength;
      onProgress(total ? done / total : null);
    }
  });
  await relaunch();
}
