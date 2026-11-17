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

# SSH on this Mac for a user of its own (a tidy home, a short prompt), with a key only the app gets.
sudo launchctl load -w /System/Library/LaunchDaemons/ssh.plist 2>/dev/null || sudo systemsetup -f -setremotelogin on
key="$PWD/build/shot_key"
rm -f "$key" "$key.pub"
ssh-keygen -q -t ed25519 -N '' -C detour-screenshots -f "$key"
user=deploy
home=/Users/$user
sudo sysadminctl -addUser "$user" -fullName Deploy -password "$(uuidgen)" -home "$home" -shell /bin/zsh
sudo createhomedir -c -u "$user" > /dev/null 2>&1 || true
sudo dseditgroup -o edit -a "$user" -t user com.apple.access_ssh 2>/dev/null || true
sudo mkdir -p "$home/.ssh" "$home/app/releases" "$home/logs" "$home/backups"
sudo cp "$key.pub" "$home/.ssh/authorized_keys"
echo "PROMPT='%n@build-mac:%~\$ '" | sudo tee "$home/.zshrc" > /dev/null
sudo tee "$home/logs/app.log" > /dev/null <<'LOG'
09:40:02 INFO  deploy started v2.14.0
09:40:05 INFO  GET /health 200 4ms
09:40:07 WARN  slow query 412ms
09:40:09 INFO  POST /api/login 200 38ms
09:40:11 ERROR GET /api/report 500 1.2s
09:40:12 INFO  db 10.0.2.5:5432 ok
09:40:14 INFO  deploy finished
LOG
sudo tee "$home/app/docker-compose.yml" > /dev/null <<'YML'
services:
  web:
    image: nginx:1.27
    ports: ["80:80"]
YML
echo "Detour sample server" | sudo tee "$home/app/README.md" > /dev/null
sudo dd if=/dev/urandom of="$home/backups/db-2026-11-16.sql.gz" bs=1m count=3 2> /dev/null
sudo dd if=/dev/urandom of="$home/app/releases/v2.14.0.tar.gz" bs=1m count=5 2> /dev/null
sudo chown -R "$user:staff" "$home"
sudo chmod 700 "$home/.ssh"
sudo chmod 600 "$home/.ssh/authorized_keys"
for _ in $(seq 1 20); do
  ssh -o BatchMode=yes -o StrictHostKeyChecking=no -i "$key" "$user@127.0.0.1" true && break
  sleep 1
done

# The newest 6.9" iPhone there is.
dev=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
d = json.load(sys.stdin)["devices"]
pro = [x for k, v in sorted(d.items()) if "iOS" in k for x in v if x["name"].endswith("Pro Max")]
print(pro[-1]["udid"])')
xcrun simctl boot "$dev" 2>/dev/null || true
xcrun simctl bootstatus "$dev" -b
xcrun simctl ui "$dev" appearance dark
xcrun simctl status_bar "$dev" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3   --cellularMode active --cellularBars 4 --batteryState charged --batteryLevel 100

# The debug app, built for that simulator.
xcodebuild -project Detour.xcodeproj -scheme Detour -configuration Debug -destination "id=$dev"   -skipPackagePluginValidation -skipMacroValidation build | xcbeautify --renderer github-actions
app=$(find ~/Library/Developer/Xcode/DerivedData -maxdepth 6 -path '*/Build/Products/Debug-iphonesimulator/Detour.app' -newer "$key" 2>/dev/null | head -1)
echo "app: $app"
ls "$app" | head -20
xcrun simctl install "$dev" "$app"

export SIMCTL_CHILD_DETOUR_SHOTS_KEY="$(cat "$key")"
export SIMCTL_CHILD_DETOUR_SHOTS_USER="$user"
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
