#!/bin/sh
# Points the Homebrew cask (github.com/jhasubhash/homebrew-tap) at this
# version's release: its version and the dmg's checksum. Run it after the
# release is published on GitHub, since the cask downloads it from there:
#
#   scripts/release.sh
#   gh release create v<version> target/dist/oneprompt-<version>.dmg --prerelease ...
#   scripts/update-cask.sh
#
# The checksum is taken from the published dmg, so it is what users get.
# TAP overrides the tap repository (owner/name).
set -e

repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)
tap=${TAP:-jhasubhash/homebrew-tap}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh release download "v$version" --repo jhasubhash/oneprompt --pattern "oneprompt-$version.dmg" --dir "$work"
sha=$(shasum -a 256 "$work/oneprompt-$version.dmg" | cut -d' ' -f1)

gh repo clone "$tap" "$work/tap" -- --quiet
cask="$work/tap/Casks/oneprompt.rb"
sed -i '' -e "s/^  version \".*\"/  version \"$version\"/" -e "s/^  sha256 \".*\"/  sha256 \"$sha\"/" "$cask"
if git -C "$work/tap" diff --quiet; then
    echo "the cask is already at $version"
    exit 0
fi
git -C "$work/tap" commit --quiet -am "Update oneprompt to $version"
git -C "$work/tap" push --quiet
echo "cask updated to $version ($sha)"
