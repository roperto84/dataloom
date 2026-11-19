#!/usr/bin/env bash
# Build Dataloom.app with its icon, optionally installing it in /Applications.
#
#   tools/package.sh            build and sign the bundle in app/dist/
#   tools/package.sh --install  also replace /Applications/Dataloom.app
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT="$(pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
BUNDLE="$ROOT/dist/Dataloom.app"
ICONSET="$ROOT/target/AppIcon.iconset"

python3 tools/make_icon.py
cargo build --release

rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for spec in \
    "16 icon_16x16" "32 icon_16x16@2x" \
    "32 icon_32x32" "64 icon_32x32@2x" \
    "128 icon_128x128" "256 icon_128x128@2x" \
    "256 icon_256x256" "512 icon_256x256@2x" \
    "512 icon_512x512" "1024 icon_512x512@2x"; do
    size="${spec%% *}"
    name="${spec##* }"
    sips -z "$size" "$size" assets/icon.png --out "$ICONSET/$name.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o target/AppIcon.icns

rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp target/release/dataloom "$BUNDLE/Contents/MacOS/dataloom"
cp target/AppIcon.icns "$BUNDLE/Contents/Resources/AppIcon.icns"

cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Dataloom</string>
    <key>CFBundleExecutable</key>
    <string>dataloom</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>com.dataloom.app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Dataloom</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION:-0.1.0}</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
PLIST

# Ad-hoc signature keeps Gatekeeper quiet for a locally built app.
codesign --force --deep --sign - "$BUNDLE" >/dev/null 2>&1 || true

echo "Built $BUNDLE"

if [ "${1:-}" = "--install" ]; then
    rm -rf "/Applications/Dataloom.app"
    ditto "$BUNDLE" "/Applications/Dataloom.app"
    echo "Installed /Applications/Dataloom.app"
fi
