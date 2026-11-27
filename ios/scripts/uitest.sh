#!/bin/bash
# Drags hosts into empty groups in the simulator (UITests/DragTests.swift) and keeps what came
# out: screenshots, where each host landed and the list's moves as the app saw them.
# Needs the core built and the project made (build-core.sh, xcodegen). Out: build/uitest/
set -uo pipefail
cd "$(dirname "$0")/.."
out="$PWD/build/uitest"
rm -rf "$out"
mkdir -p "$out"

phone=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
d = json.load(sys.stdin)["devices"]
found = [x for k, v in sorted(d.items()) if "iOS" in k for x in v if "iPhone 17 Pro" in x["name"] or "iPhone 16 Pro" in x["name"]]
print(found[-1]["udid"] if found else "")')
echo "simulator: $phone"
xcrun simctl boot "$phone" 2>/dev/null || true
xcrun simctl bootstatus "$phone" -b

TEST_RUNNER_DRAG_OUT="$out" xcodebuild test -project Detour.xcodeproj -scheme Detour -configuration Debug \
  -destination "id=$phone" -only-testing:DetourUITests \
  -skipPackagePluginValidation -skipMacroValidation CODE_SIGNING_ALLOWED=NO 2>&1 | tee "$out/xcodebuild.log" | xcbeautify --renderer github-actions
status=${PIPESTATUS[0]}

data=$(xcrun simctl get_app_container "$phone" dev.detour.ios data 2>/dev/null || true)
[ -n "$data" ] && cp "$data/Documents/drag.txt" "$out/" 2>/dev/null
echo "== result"; cat "$out/result.txt" 2>/dev/null
echo "== moves"; cat "$out/drag.txt" 2>/dev/null
ls -la "$out"
exit 0
