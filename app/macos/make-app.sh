#!/bin/sh
# Build QuipKeys.app (a menu-bar app, no Dock icon) into app/dist/.
# Usage, from the app/ folder:  sh macos/make-app.sh
# Then drag dist/QuipKeys.app into Applications and open it.
set -e
cd "$(dirname "$0")/.."
cargo build --release
APP=dist/QuipKeys.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/quipkeys "$APP/Contents/MacOS/quipkeys"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>QuipKeys</string>
  <key>CFBundleDisplayName</key><string>QuipKeys</string>
  <key>CFBundleIdentifier</key><string>com.quipkeys.app</string>
  <key>CFBundleExecutable</key><string>quipkeys</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature, so macOS remembers the Accessibility permission between runs.
codesign --force --deep --sign - "$APP"
echo "Built $APP"
