"""Builds Detour's MCP bundle (`detour-<version>.mcpb`) from the command line archives of a
release, adds it to that release and points `server.json` (the MCP Registry entry) at it.

    python scripts/mcpb.py v0.6.0 [--no-upload]

Run it after the release has all the `detour-cli-*` archives (Windows' is added by
scripts/release-windows.ps1, which calls this at the end). Then publish the registry entry:

    mcp-publisher login github
    mcp-publisher publish server.json

The bundle holds `detour` for every system and a launcher (packaging/mcpb) that runs the
Detour installed on the computer when there is one: the core it starts then knows the
Detour window, where the person approves an agent's access.
"""

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = "INotSleep/Detour"
SRC = os.path.join(ROOT, "packaging", "mcpb")

# Archive suffix -> where its `detour` goes in the bundle.
TARGETS = {
    "windows-x86_64.zip": "server/win32/detour.exe",
    "macos.tar.gz": "server/darwin/detour",
    "linux-x86_64.tar.gz": "server/linux-x64/detour",
    "linux-arm64.tar.gz": "server/linux-arm64/detour",
}


def run(*cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def fetch(tag, version, work):
    """The release's command line archives, checked against their .sha256."""
    run("gh", "release", "download", tag, "-R", REPO, "-p", f"detour-cli-{version}-*", "-D", work, "--clobber")
    out = {}
    for suffix in TARGETS:
        name = f"detour-cli-{version}-{suffix}"
        path = os.path.join(work, name)
        want = open(path + ".sha256", encoding="utf-8").read().split()[0].lower()
        got = hashlib.sha256(open(path, "rb").read()).hexdigest()
        if got != want:
            sys.exit(f"{name}: checksum doesn't match")
        out[suffix] = path
    return out


def binary(archive):
    """The `detour` inside a command line archive."""
    if archive.endswith(".zip"):
        with zipfile.ZipFile(archive) as z:
            return z.read("detour.exe")
    with tarfile.open(archive) as t:
        return t.extractfile("detour").read()


def tools(exe):
    """The tools `detour mcp` offers, for the manifest: name and first sentence."""
    msgs = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mcpb", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    ]
    with tempfile.TemporaryDirectory() as vault:
        p = subprocess.Popen([exe, "--data-dir", vault, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8")
        p.stdin.write(json.dumps(msgs[0]) + "\n")
        p.stdin.flush()
        p.stdout.readline()
        for m in msgs[1:]:
            p.stdin.write(json.dumps(m) + "\n")
        p.stdin.flush()
        listed = json.loads(p.stdout.readline())["result"]["tools"]
        p.stdin.close()
        p.wait(timeout=15)
    return [{"name": t["name"], "description": t["description"].split(". ")[0].rstrip(".") + "."} for t in listed]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) != 1:
        sys.exit(__doc__)
    tag = args[0]
    version = tag.lstrip("v")
    upload = "--no-upload" not in sys.argv
    work = tempfile.mkdtemp(prefix="detour-mcpb-")
    try:
        archives = fetch(tag, version, work)
        files = {dest: binary(archives[suffix]) for suffix, dest in TARGETS.items()}

        # The tools come from this computer's own copy, so it runs here.
        local = {"win32": "server/win32/detour.exe", "darwin": "server/darwin/detour"}.get(sys.platform, "server/linux-x64/detour")
        exe = os.path.join(work, "detour-local" + (".exe" if sys.platform == "win32" else ""))
        open(exe, "wb").write(files[local])
        os.chmod(exe, 0o755)

        manifest = json.load(open(os.path.join(SRC, "manifest.json"), encoding="utf-8"))
        manifest["version"] = version
        manifest["tools"] = tools(exe)

        out = os.path.join(work, f"detour-{version}.mcpb")
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            def add(name, data, mode):
                info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.create_system = 3  # unix, so the modes below count
                info.external_attr = (0o100000 | mode) << 16
                z.writestr(info, data)

            add("manifest.json", json.dumps(manifest, indent=2).encode() + b"\n", 0o644)
            add("icon.png", open(os.path.join(SRC, "icon.png"), "rb").read(), 0o644)
            # Line endings as the launchers need them, whatever the checkout did.
            sh = open(os.path.join(SRC, "launch.sh"), "rb").read().replace(b"\r\n", b"\n")
            cmd = open(os.path.join(SRC, "launch.cmd"), "rb").read().replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
            add("server/launch.sh", sh, 0o755)
            add("server/launch.cmd", cmd, 0o644)
            for dest, data in files.items():
                add(dest, data, 0o755)

        data = open(out, "rb").read()
        sha = hashlib.sha256(data).hexdigest()
        print(f"{os.path.basename(out)}: {len(data) // 1048576} MB, sha256 {sha}, {len(manifest['tools'])} tools")

        if upload:
            run("gh", "release", "upload", tag, out, "-R", REPO, "--clobber")
        else:
            shutil.copy(out, ROOT)

        path = os.path.join(ROOT, "server.json")
        entry = json.load(open(path, encoding="utf-8"))
        entry["version"] = version
        pkg = entry["packages"][0]
        pkg.update(identifier=f"https://github.com/{REPO}/releases/download/{tag}/{os.path.basename(out)}", version=version, fileSha256=sha)
        open(path, "w", encoding="utf-8", newline="\n").write(json.dumps(entry, indent=2) + "\n")
        print("server.json points at it. Publish: mcp-publisher login github && mcp-publisher publish server.json")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
