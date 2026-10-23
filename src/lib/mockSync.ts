// Personal vault sync in a plain browser (no detour-core): enough to show every state.
import type { CoreError, Pairing, SyncInfo, SyncPlace } from "./api";

const off: SyncInfo = { enabled: false, location: null, git: false, pending: false, error: null, syncedAt: null, problems: [] };
let state: SyncInfo = { ...off };

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const chars = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const group = () => Array.from({ length: 4 }, () => chars[Math.floor(Math.random() * chars.length)]).join("");

function placed(place: SyncPlace): SyncInfo {
  const git = !("path" in place);
  return { ...off, enabled: true, git, location: "path" in place ? place.path : place.url, syncedAt: Date.now() };
}

export const demoSync = {
  async status() {
    return state;
  },
  async now() {
    await wait(600);
    if (state.enabled) state = { ...state, syncedAt: Date.now(), pending: false, error: null };
    return { changed: false, status: state };
  },
  async enable(place: SyncPlace) {
    await wait(700);
    state = placed(place);
    return state;
  },
  async join(place: SyncPlace, code: string) {
    await wait(900);
    if (!/^[0-9A-Z]{4}(-?[0-9A-Z]{4}){3}$/i.test(code.trim()) && !/^DETOUR-RECOVERY/i.test(code.trim())) throw { kind: "invalidCode" } satisfies CoreError;
    state = placed(place);
    return state;
  },
  async addDevice(): Promise<Pairing> {
    await wait(800);
    return { code: [group(), group(), group(), group()].join("-"), expiresAt: Date.now() + 60 * 60 * 1000 };
  },
  async recoveryKey() {
    return `DETOUR-RECOVERY-${Array.from({ length: 28 }, group).join("-")}`;
  },
  async disable() {
    state = { ...off };
  },
};
