// Builds the command line (`detour`) and puts it where Tauri takes sidecars from:
// src-tauri/binaries/detour-<target triple>[.exe]. Installers then ship it next to the
// app (/usr/bin/detour, Detour.app/Contents/MacOS/detour, the install folder on Windows).
//
// Run by `tauri build` (beforeBuildCommand of tauri.bundle.conf.json). The target is the
// one Tauri builds for (TAURI_ENV_TARGET_TRIPLE). A universal macOS build needs one for
// each architecture (Tauri builds the app once for each) and both joined with lipo.
// DETOUR_SIDECAR_READY=1 skips it: the release script builds and signs the sidecar first.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
if (process.env.DETOUR_SIDECAR_READY === "1") process.exit(0);

const host = /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const exe = triple.includes("windows") ? ".exe" : "";
const out = join(root, "src-tauri", "binaries", `detour-${triple}${exe}`);
mkdirSync(dirname(out), { recursive: true });

const build = (target) => {
  const args = ["build", "-p", "detour-cli", "--release", "--locked"];
  if (target !== host) args.push("--target", target);
  execFileSync("cargo", args, { cwd: root, stdio: "inherit" });
  return join(root, "target", target === host ? "" : target, "release", `detour${exe}`);
};

if (triple === "universal-apple-darwin") {
  const arches = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
  const parts = arches.map(build);
  arches.forEach((a, i) => copyFileSync(parts[i], join(dirname(out), `detour-${a}`)));
  execFileSync("lipo", ["-create", "-output", out, ...parts], { stdio: "inherit" });
} else {
  copyFileSync(build(triple), out);
}
console.log(`sidecar: ${out}`);
