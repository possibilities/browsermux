#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Darwin ]] || { echo 'Native build requires macOS and an official Xcode SDK' >&2; exit 64; }
[[ $(uname -m) == arm64 ]] || { echo 'This alpha currently targets Apple Silicon' >&2; exit 64; }
export MACOSX_DEPLOYMENT_TARGET=14.5
xcrun --find swiftc >/dev/null
# Catch native syntax failures before fetching and building dependencies.
xcrun swiftc -frontend -parse macos/Sources/*.swift
python3 scripts/fetch-cef.py
cargo build --locked --release -p browser-bridge
mkdir -p build/generated
cargo run --locked --release -p browser-bridge --bin uniffi-bindgen -- generate --library target/release/libbrowser_bridge.dylib --language swift --out-dir build/generated
xcrun swiftc -typecheck -swift-version 5 -target arm64-apple-macosx14.5 \
  -import-objc-header macos/CEF/BrowserEngine.h -I build/generated -Xcc -fmodule-map-file=build/generated/browser_bridgeFFI.modulemap \
  build/generated/browser_bridge.swift macos/Sources/*.swift
cmake -S macos -B build/native -DCMAKE_BUILD_TYPE=Release -DPROJECT_ARCH=arm64 -DCMAKE_OSX_ARCHITECTURES=arm64
cmake --build build/native --parallel 3
APP="$PWD/build/browsermux.app"
mkdir -p "$APP/Contents/"{MacOS,Frameworks,Resources}
cp macos/Resources/Info.plist "$APP/Contents/Info.plist"
xcrun swiftc -swift-version 5 -O -g -target arm64-apple-macosx14.5 \
  -import-objc-header macos/CEF/BrowserEngine.h -I build/generated -Xcc -fmodule-map-file=build/generated/browser_bridgeFFI.modulemap \
  build/generated/browser_bridge.swift macos/Sources/*.swift \
  target/release/libbrowser_bridge.a build/native/libspb_engine.a \
  build/native/libcef_dll_wrapper/libcef_dll_wrapper.a \
  -framework AppKit -framework Security -framework CoreFoundation -framework SystemConfiguration -framework IOSurface \
  -lc++ -liconv -Xlinker -ObjC -o "$APP/Contents/MacOS/browsermux"
ditto 'vendor/cef/Release/Chromium Embedded Framework.framework' "$APP/Contents/Frameworks/Chromium Embedded Framework.framework"
FRAMEWORK="$APP/Contents/Frameworks/Chromium Embedded Framework.framework"
# Sign nested binaries inside-out. Do not deep-resign helpers and strip JIT entitlements.
while IFS= read -r -d '' library; do codesign --force --sign - "$library"; done < <(find "$FRAMEWORK" -type f -name '*.dylib' -print0)
codesign --force --sign - "$FRAMEWORK"
for suffix in '' ' (Alerts)' ' (GPU)' ' (Plugin)' ' (Renderer)'; do
  NAME="browsermux Helper$suffix"
  HELPER="$APP/Contents/Frameworks/$NAME.app"
  mkdir -p "$HELPER/Contents/MacOS"
  cp build/native/spb_helper "$HELPER/Contents/MacOS/$NAME"
  python3 - "$HELPER/Contents/Info.plist" "$NAME" <<'PY'
import plistlib,sys
name=sys.argv[2]
with open(sys.argv[1],'wb') as f:plistlib.dump({'CFBundleName':name,'CFBundleExecutable':name,'CFBundleIdentifier':'com.possibilities.browsermux.helper.'+name.replace(' ','').replace('(','').replace(')','').lower(),'CFBundlePackageType':'APPL','LSUIElement':True},f)
PY
  codesign --force --sign - --entitlements macos/Resources/helper-entitlements.plist "$HELPER"
done
cp vendor/cef/LICENSE.txt "$APP/Contents/Resources/CEF-LICENSE.txt"
cp vendor/cef/CREDITS.html "$APP/Contents/Resources/Chromium-CREDITS.html"
# Ad-hoc local test signature only: this is NOT Developer ID or notarization.
codesign --force --sign - "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"
echo "$APP"
