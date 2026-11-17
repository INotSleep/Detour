#!/bin/bash
# App Store screenshots from the simulator (a 6.9" iPhone, dark): the debug app fills a vault
# of its own with sample hosts (`-shot`, see App/Shots.swift) and opens each screen; the
# terminal and the files are real, over SSH to this Mac. Needs the core built and the project
# made (build-core.sh, xcodegen). Out: build/screenshots/*.png
set -euo pipefail
cd "$(dirname "$0")/.."
out="$PWD/build/screenshots"
rm -rf "$out"
mkdir -p "$out"

# SSH on this Mac, with a key only the app gets.
sudo launchctl load -w /System/Library/LaunchDaemons/ssh.plist 2>/dev/null || sudo systemsetup -f -setremotelogin on
key="$PWD/build/shot_key"
rm -f "$key" "$key.pub"
ssh-keygen -q -t ed25519 -N '' -C detour-screenshots -f "$key"
mkdir -p ~/.ssh
cat "$key.pub" >> ~/.ssh/authorized_keys
chmod 700 ~/.ssh
chmod 600 ~/.ssh/authorized_keys
for _ in $(seq 1 20); do
  ssh -o BatchMode=yes -o StrictHostKeyChecking=no -i "$key" "$USER@127.0.0.1" true && break
  sleep 1
done

# The debug build for the simulator (the workflow's simulator build made it already).
find_app() { find ~/Library/Developer/Xcode/DerivedData -maxdepth 6 -path '*/Build/Products/Debug-iphonesimulator/Detour.app' 2>/dev/null | head -1; }
app=$(find_app)
if [ -z "$app" ]; then
  xcodebuild -project Detour.xcodeproj -scheme Detour -configuration Debug \
    -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
    -skipPackagePluginValidation -skipMacroValidation CODE_SIGNING_ALLOWED=NO build | xcbeautify --renderer github-actions
  app=$(find_app)
fi
echo "app: $app"

# The newest 6.9" iPhone there is.
dev=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
d = json.load(sys.stdin)["devices"]
pro = [x for k, v in sorted(d.items()) if "iOS" in k for x in v if x["name"].endswith("Pro Max")]
print(pro[-1]["udid"])')
xcrun simctl boot "$dev" 2>/dev/null || true
xcrun simctl bootstatus "$dev" -b
xcrun simctl ui "$dev" appearance dark
xcrun simctl status_bar "$dev" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3 \
  --cellularMode active --cellularBars 4 --batteryState charged --batteryLevel 100
xcrun simctl install "$dev" "$app"

export SIMCTL_CHILD_DETOUR_SHOTS_KEY="$(cat "$key")"
export SIMCTL_CHILD_DETOUR_SHOTS_USER="$USER"
n=1
for screen in hosts terminal files snippets keys; do
  xcrun simctl terminate "$dev" dev.detour.ios 2>/dev/null || true
  xcrun simctl launch "$dev" dev.detour.ios -shot "$screen" -AppleLanguages "(en)" -AppleLocale en_US
  # The first launch makes the vault (and its key) first.
  sleep $([ "$screen" = hosts ] && echo 15 || echo 9)
  xcrun simctl io "$dev" screenshot "$out/$n-$screen.png"
  n=$((n + 1))
done
ls -la "$out"
