#!/bin/sh
# Builds a signed, notarized oneprompt.dmg for a GitHub release, in
# target/dist. The app inside is self-contained (unlike make-app.sh's, which
# links to the checkout's build) and universal: Apple silicon and Intel.
#
#   scripts/release.sh
#
# Needs, once:
#   rustup target add x86_64-apple-darwin
#   xcrun notarytool store-credentials oneprompt-notary \
#       --apple-id <your Apple ID> --team-id <team ID>
#     (asks for an app-specific password from account.apple.com)
#
# SIGN_IDENTITY overrides the signing certificate (default: the first
# "Developer ID Application" one in the keychain); NOTARY_PROFILE the
# notarytool profile; NOTARIZE=0 skips notarizing, for a local check.
set -e

repo=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# macOS wants a plain x.y.z for the bundle; the full version (0.1.0-alpha.1)
# goes in the dmg's name and the release tag.
short=${version%%-*}
identity=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}
profile=${NOTARY_PROFILE:-oneprompt-notary}
if [ -z "$identity" ]; then
    echo "no Developer ID Application certificate in the keychain" >&2
    exit 1
fi

# One executable for both kinds of Mac.
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --target "$target"
done
dist="$repo/target/dist"
rm -rf "$dist"
mkdir -p "$dist/stage"
app="$dist/stage/oneprompt.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/oneprompt" \
    target/aarch64-apple-darwin/release/oneprompt \
    target/x86_64-apple-darwin/release/oneprompt

# Icon: every size macOS asks for, from the 1024px master.
iconset=$(mktemp -d)/oneprompt.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size assets/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double assets/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
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
    <key>CFBundleShortVersionString</key><string>$short</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>Copyright 2026 Subhash Jha</string>
</dict>
</plist>
PLIST

# Hardened runtime and a secure timestamp: both required for notarizing.
codesign --force --options runtime --timestamp --sign "$identity" "$app"
codesign --verify --strict --verbose=2 "$app"

# The dmg opens to the app beside a link to Applications, to drag it across.
ln -s /Applications "$dist/stage/Applications"
dmg="$dist/oneprompt-$version.dmg"
hdiutil create -volname "oneprompt $version" -srcfolder "$dist/stage" -ov -format UDZO "$dmg" >/dev/null
codesign --force --timestamp --sign "$identity" "$dmg"

if [ "${NOTARIZE:-1}" != 0 ]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait
    # Staple the ticket, so Gatekeeper accepts the app offline too.
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
fi
rm -rf "$dist/stage"
echo "made $dmg"
