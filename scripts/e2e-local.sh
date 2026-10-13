#!/usr/bin/env bash
# End-to-end smoke test on one machine: signaling server + host + viewer (ping over WebRTC).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -q -p signaling -p peer-poc
tmp=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; wait 2>/dev/null; rm -rf "$tmp"' EXIT
export NO_COLOR=1

./target/debug/signaling --listen 127.0.0.1:18080 >"$tmp/server.log" 2>&1 &
sleep 1
./target/debug/peer-poc --server ws://127.0.0.1:18080/ws --key-file "$tmp/host.key" host --source pattern --personal-password "Personale-123" >"$tmp/host.log" 2>&1 &

# The host prints its password once ready (after deriving the personal secret).
for _ in $(seq 100); do
    password=$(sed -n 's/^Password: //p' "$tmp/host.log")
    [ -n "$password" ] && break
    sleep 0.1
done
id=$(sed -n 's/^Registered as //p' "$tmp/host.log")
[ -n "${password:-}" ] || { echo "host not ready"; cat "$tmp/host.log"; exit 1; }

echo "== wrong password must be refused"
if ./target/debug/peer-poc --server ws://127.0.0.1:18080/ws connect "$id" -p wrongpw1 --seconds 1; then
    echo "FAIL: wrong password accepted"; exit 1
fi

echo "== right password: receive the test pattern stream"
./target/debug/peer-poc --server ws://127.0.0.1:18080/ws connect "$id" -p "$password" --seconds 4 --save "$tmp/stream.h264"
if command -v ffprobe >/dev/null; then
    ffprobe -v error -count_frames -select_streams v:0 \
        -show_entries stream=codec_name,width,height,nb_read_frames -of csv=p=0 "$tmp/stream.h264"
fi

echo "== personal password (unattended): accepted without approval"
./target/debug/peer-poc --server ws://127.0.0.1:18080/ws connect "$id" -p "Personale-123" --personal --seconds 2
grep -q "Viewer authenticated (Unattended)" "$tmp/host.log"

echo "== wrong personal password must be refused"
if ./target/debug/peer-poc --server ws://127.0.0.1:18080/ws connect "$id" -p "personale-123" --personal --seconds 1; then
    echo "FAIL: wrong personal password accepted"; exit 1
fi
