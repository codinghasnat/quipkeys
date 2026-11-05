#!/bin/sh
# Build QuipKeys.app (a menu-bar app, no Dock icon) into app/dist/.
# Usage, from the app/ folder:
#   sh macos/make-app.sh            build dist/QuipKeys.app
#   sh macos/make-app.sh --install  build, then replace /Applications/QuipKeys.app
#                                   and start it (use this for updates)
set -e
cd "$(dirname "$0")/.."
cargo build --release
APP=dist/QuipKeys.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/quipkeys "$APP/Contents/MacOS/quipkeys"
cp macos/QuipKeys.icns "$APP/Contents/Resources/QuipKeys.icns"
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
  <key>CFBundleIconFile</key><string>QuipKeys</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature (Apple silicon won't run unsigned code).
codesign --force --deep --sign - "$APP"
echo "Built $APP"

if [ "$1" = "--install" ]; then
  # Quit the running copy (if any) and put the new one in place.
  pkill -x quipkeys 2>/dev/null && sleep 1 || true
  rm -rf /Applications/QuipKeys.app
  cp -R "$APP" /Applications/QuipKeys.app
  # Each ad-hoc-signed build looks like a new app to macOS, so the old
  # Accessibility entry no longer applies: clear it so macOS asks afresh.
  tccutil reset Accessibility com.quipkeys.app >/dev/null 2>&1 || true
  open /Applications/QuipKeys.app
  echo "Installed and started /Applications/QuipKeys.app."
  echo "macOS will ask for Accessibility permission again: switch QuipKeys on in"
  echo "System Settings > Privacy & Security > Accessibility (the welcome window links there)."
fi
