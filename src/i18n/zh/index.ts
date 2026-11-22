// Simplified Chinese.
import type { Dict } from "../en";
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

const dict: Dict = { common, app, agents, settings, sync, teams, files, terminal, hosts, keys, tunnels, proxies, demo };
export default dict;

/** The language's own name, and the locale for dates and numbers. */
export const meta = { name: "简体中文", locale: "zh-CN" };
