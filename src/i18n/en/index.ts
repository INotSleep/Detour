// English: the reference every other language follows key for key.
import agents from "./agents";
import app from "./app";
import common from "./common";
import demo from "./demo";
import files from "./files";
import hosts from "./hosts";
import keys from "./keys";
import proxies from "./proxies";
import settings from "./settings";
import sync from "./sync";
import teams from "./teams";
import terminal from "./terminal";
import tunnels from "./tunnels";

const dict = { common, app, agents, settings, sync, teams, files, terminal, hosts, keys, tunnels, proxies, demo };
export type Dict = typeof dict;
export default dict;

/** The language's own name, and the locale for dates and numbers. */
export const meta = { name: "English", locale: "en-US" };
