#!/bin/bash
# An ad hoc build for the devices in a provisioning profile, signed with a distribution
# certificate. In CI from secrets (base64):
#   IOS_CERTIFICATE_P12_BASE64, IOS_CERTIFICATE_PASSWORD, IOS_PROFILE_BASE64
# The app takes the profile's bundle id (or dev.detour.ios under a wildcard one) and team.
set -euo pipefail
cd "$(dirname "$0")/.."
work="$PWD/build/signing"
rm -rf "$work" build/archive.xcarchive build/export
mkdir -p "$work"

echo "$IOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$work/cert.p12"
echo "$IOS_PROFILE_BASE64" | base64 --decode > "$work/profile.mobileprovision"

# A keychain of its own for the certificate, gone with the runner.
keychain="$work/signing.keychain-db"
kc_pass="$(uuidgen)"
security create-keychain -p "$kc_pass" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$kc_pass" "$keychain"
security import "$work/cert.p12" -P "$IOS_CERTIFICATE_PASSWORD" -A -t cert -f pkcs12 -k "$keychain"
security set-key-partition-list -S apple-tool:,apple: -s -k "$kc_pass" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | sed 's/"//g')

security cms -D -i "$work/profile.mobileprovision" > "$work/profile.plist"
uuid=$(/usr/libexec/PlistBuddy -c "Print UUID" "$work/profile.plist")
team=$(/usr/libexec/PlistBuddy -c "Print TeamIdentifier:0" "$work/profile.plist")
name=$(/usr/libexec/PlistBuddy -c "Print Name" "$work/profile.plist")
appid=$(/usr/libexec/PlistBuddy -c "Print Entitlements:application-identifier" "$work/profile.plist")
bundle="${appid#"$team".}"
[ "$bundle" = "*" ] && bundle="dev.detour.ios"
mkdir -p "$HOME/Library/MobileDevice/Provisioning Profiles"
cp "$work/profile.mobileprovision" "$HOME/Library/MobileDevice/Provisioning Profiles/$uuid.mobileprovision"
echo "profile: $name, team $team, bundle $bundle"

xcodebuild -project Detour.xcodeproj -scheme Detour -configuration Release \
  -destination 'generic/platform=iOS' -archivePath build/archive.xcarchive \
  -skipPackagePluginValidation -skipMacroValidation \
  DEVELOPMENT_TEAM="$team" DETOUR_BUNDLE_ID="$bundle" \
  CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY="Apple Distribution" PROVISIONING_PROFILE_SPECIFIER="$uuid" \
  OTHER_CODE_SIGN_FLAGS="--keychain $keychain" \
  archive | xcbeautify --renderer github-actions

cat > "$work/export.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>release-testing</string>
  <key>teamID</key><string>$team</string>
  <key>signingStyle</key><string>manual</string>
  <key>provisioningProfiles</key><dict><key>$bundle</key><string>$uuid</string></dict>
  <key>compileBitcode</key><false/>
  <key>stripSwiftSymbols</key><true/>
</dict>
</plist>
EOF
xcodebuild -exportArchive -archivePath build/archive.xcarchive -exportPath build/export -exportOptionsPlist "$work/export.plist"
ls -la build/export
