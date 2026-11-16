#!/bin/bash
# Builds Detour's core for iOS (device and simulator) as Frameworks/DetourCore.xcframework,
# and its Swift side into Generated/. Run on a Mac with the iOS Rust targets:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim
#   ios/scripts/build-core.sh [release|debug]
set -euo pipefail
cd "$(dirname "$0")/../.."
profile="${1:-release}"
flag=$([ "$profile" = release ] && echo --release || true)
ios="$PWD/ios"

export IPHONEOS_DEPLOYMENT_TARGET=17.0
for target in aarch64-apple-ios aarch64-apple-ios-sim; do
  cargo build -p detour-mobile --lib $flag --target "$target"
done

# The Swift side comes from the library's own description of itself.
cargo build -p detour-mobile --lib $flag
rm -rf "$ios/Generated" "$ios/build/include"
mkdir -p "$ios/Generated" "$ios/build/include"
cargo run -p detour-mobile --features bindgen --bin uniffi-bindgen -- \
  generate --library "target/$profile/libdetour_mobile.dylib" --language swift --out-dir "$ios/build/bindings"
mv "$ios/build/bindings/detour_mobile.swift" "$ios/Generated/"
cp "$ios/build/bindings/detour_mobileFFI.h" "$ios/build/include/"
cp "$ios/build/bindings/detour_mobileFFI.modulemap" "$ios/build/include/module.modulemap"

rm -rf "$ios/Frameworks/DetourCore.xcframework"
mkdir -p "$ios/Frameworks"
xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/$profile/libdetour_mobile.a" -headers "$ios/build/include" \
  -library "target/aarch64-apple-ios-sim/$profile/libdetour_mobile.a" -headers "$ios/build/include" \
  -output "$ios/Frameworks/DetourCore.xcframework"
echo "core: $ios/Frameworks/DetourCore.xcframework"
