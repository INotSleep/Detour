// The only place the UI talks to the backend. In the Tauri window calls go to
// detour-core through Tauri commands; in a plain browser (vite dev) they fall
// back to mock data, so the UI can be developed without the Rust side.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { mockWorkspace } from "./mockData";
import type { Workspace } from "./types";

export interface CoreInfo {
  coreVersion: string;
  os: string;
}

export const api = {
  async coreInfo(): Promise<CoreInfo | null> {
    return isTauri() ? invoke<CoreInfo>("core_info") : null;
  },

  // Still mock in both modes: storage moves into detour-core in a later step.
  async loadWorkspace(): Promise<Workspace> {
    return structuredClone(mockWorkspace);
  },

  /** Asks where to save and writes the file. Returns where it went, or null if cancelled. */
  async saveTextFile(defaultName: string, contents: string): Promise<string | null> {
    if (isTauri()) {
      const path = await save({ defaultPath: defaultName });
      if (!path) return null;
      await invoke("write_text_file", { path, contents });
      return path;
    }
    const url = URL.createObjectURL(new Blob([contents], { type: "text/plain" }));
    const a = Object.assign(document.createElement("a"), { href: url, download: defaultName });
    a.click();
    URL.revokeObjectURL(url);
    return defaultName;
  },
};
