#!/bin/bash
# Builds Detour's core for Android (phones and the x86_64 emulator) into app/src/main/jniLibs,
# and its Kotlin side into app/src/main/java/uniffi. Needs the NDK and cargo-ndk:
#   rustup target add aarch64-linux-android x86_64-linux-android
#   cargo install cargo-ndk
#   android/scripts/build-core.sh [release|debug] [abi...]
set -euo pipefail
cd "$(dirname "$0")/../.."
profile="${1:-release}"
shift || true
abis=("${@:-arm64-v8a x86_64}")
flag=$([ "$profile" = release ] && echo --release || true)
android="$PWD/android"
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  sdk="${ANDROID_HOME:-${LOCALAPPDATA:-$HOME/Library}/Android/Sdk}"
  export ANDROID_NDK_HOME="$sdk/ndk/$(ls "$sdk/ndk" | sort -V | tail -1)"
fi

targets=()
for abi in ${abis[@]}; do targets+=(-t "$abi"); done
cargo ndk "${targets[@]}" -P 26 -o "$android/app/src/main/jniLibs" build -p detour-mobile --lib $flag
# cargo-ndk copies every dylib of the build; the core is all in one library.
rm -f "$android"/app/src/main/jniLibs/*/libboringtun-*.so

# The Kotlin side comes from the library's own description of itself.
lib=$(ls "$android"/app/src/main/jniLibs/*/libdetour_mobile.so | head -1)
rm -rf "$android/app/src/main/java/uniffi"
cargo run -p detour-mobile --features bindgen --bin uniffi-bindgen -- \
  generate --library "$lib" --language kotlin --no-format --out-dir "$android/app/src/main/java"
echo "core: $android/app/src/main/jniLibs"
