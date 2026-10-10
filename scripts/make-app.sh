#!/bin/sh
# Builds oneprompt.app in ~/Applications (or the directory given), for
# launching from Spotlight, Launchpad or the Dock.
#
# The app's executable is a symlink to this checkout's release build, so it
# never needs remaking: every scripts/build.sh is what the app runs next time
# it opens (build.sh signs it, so macOS keeps the permissions granted to it;
# a plain `cargo build --release` works too but loses them). Rerun this only
# to move the app, change its icon or its Info.plist.
set -e

repo=$(cd "$(dirname "$0")/.." && pwd)
dest=${1:-$HOME/Applications}
app="$dest/oneprompt.app"

"$repo/scripts/build.sh"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
ln -s "$repo/target/release/oneprompt" "$app/Contents/MacOS/oneprompt"

# Icon: every size macOS asks for, from the 1024px master.
iconset=$(mktemp -d)/oneprompt.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size "$repo/assets/icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$repo/assets/icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/oneprompt.icns"
rm -rf "$(dirname "$iconset")"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>oneprompt</string>
    <key>CFBundleDisplayName</key><string>oneprompt</string>
    <key>CFBundleIdentifier</key><string>dev.oneprompt.terminal</string>
    <key>CFBundleExecutable</key><string>oneprompt</string>
    <key>CFBundleIconFile</key><string>oneprompt</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
$(cat "$repo/scripts/usage-strings.plist")
</dict>
</plist>
PLIST

# Tell Launch Services (Spotlight, Dock) about the new bundle.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
echo "made $app -> $repo/target/release/oneprompt"
