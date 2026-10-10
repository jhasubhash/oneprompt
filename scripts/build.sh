#!/bin/sh
# Builds the release binary that ~/Applications/oneprompt.app runs, and
# signs it with a fixed identity, so macOS knows it as the same app after
# every build.
#
# Why: privacy permissions (Photos, folders, ...) that programs run inside
# the terminal ask for are granted to the terminal app, keyed by its code
# signature. The linker's ad-hoc signature changes with every build, so
# macOS took each build for a new app and asked again on every launch.
# Signed with the Developer ID certificate and the bundle's identifier,
# it stays the same app. Without that certificate (another machine) the
# build stays ad-hoc signed and still runs.
set -e
repo=$(cd "$(dirname "$0")/.." && pwd)
cargo build --release --manifest-path "$repo/Cargo.toml" "$@"
identity=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)}
if [ -n "$identity" ]; then
    codesign --force --sign "$identity" --identifier dev.oneprompt.terminal "$repo/target/release/oneprompt"
else
    echo "build.sh: no Developer ID certificate; left ad-hoc signed (permissions are asked again after each build)" >&2
fi
