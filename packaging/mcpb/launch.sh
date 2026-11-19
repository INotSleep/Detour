#!/bin/sh
# Runs the MCP server of the Detour installed here, so the core it starts knows the
# Detour window; the detour in this bundle only when Detour is not installed.
for d in "/Applications/Detour.app/Contents/MacOS/detour" "$HOME/Applications/Detour.app/Contents/MacOS/detour"; do
  [ -x "$d" ] && exec "$d" mcp
done
d=$(command -v detour 2>/dev/null) && [ -n "$d" ] && exec "$d" mcp
here=$(cd "$(dirname "$0")" && pwd)
if [ "$(uname -s)" = Darwin ]; then exec "$here/darwin/detour" mcp; fi
case "$(uname -m)" in
  aarch64|arm64) exec "$here/linux-arm64/detour" mcp ;;
  *) exec "$here/linux-x64/detour" mcp ;;
esac
