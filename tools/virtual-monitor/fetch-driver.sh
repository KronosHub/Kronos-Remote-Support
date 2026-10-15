#!/usr/bin/env bash
# Downloads Virtual Display Driver (MIT, signed by SignPath Foundation) into driver/,
# checking it against the release we tested; the files are embedded by the build.
set -euo pipefail
cd "$(dirname "$0")"
VERSION=25.7.23
SHA256=e24210692b442b39af763536330ce78b423f19342b7a7792c26de3944e418b3a
URL="https://github.com/VirtualDrivers/Virtual-Display-Driver/releases/download/$VERSION/VirtualDisplayDriver-x86.Driver.Only.zip"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/vdd.zip" "$URL"
actual=$( (sha256sum "$tmp/vdd.zip" 2>/dev/null || shasum -a 256 "$tmp/vdd.zip") | cut -d' ' -f1)
if [ "$actual" != "$SHA256" ]; then
    echo "checksum mismatch: $actual" >&2
    exit 1
fi
unzip -q -o "$tmp/vdd.zip" -d "$tmp/x"
rm -rf driver
mkdir driver
cp "$tmp/x/VirtualDisplayDriver/"{MttVDD.inf,MttVDD.dll,mttvdd.cat,vdd_settings.xml} driver/
echo "$VERSION" > driver/VERSION
echo "Virtual Display Driver $VERSION in $(pwd)/driver"
