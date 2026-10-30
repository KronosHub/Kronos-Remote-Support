#!/usr/bin/env bash
# Shared clipboard end-to-end on Linux: two virtual X displays have separate clipboards,
# so a host (:97) and a viewer (:98) on one machine behave like two computers.
# Needs Xvfb and xclip. Used by CI; does not touch the desktop's own clipboard.
set -euo pipefail
cd "$(dirname "$0")/.."

source scripts/worker-dev.sh
cargo build -q --release -p peer-poc
tmp=$(mktemp -d)
cleanup() {
    status=$?
    if [ $status -ne 0 ]; then
        echo "---- viewer log"; grep -vE "mdns|Failed to write" "$tmp/viewer.log" | tail -40 || true
        echo "---- host log"; grep -vE "mdns|Failed to write" "$tmp/host.log" | tail -40 || true
    fi
    kill $(jobs -p) 2>/dev/null; wait 2>/dev/null; rm -rf "$tmp"
}
trap cleanup EXIT
export NO_COLOR=1 XDG_SESSION_TYPE=x11
bin=./target/release

Xvfb :98 -screen 0 640x480x24 -noreset &
Xvfb :97 -screen 0 640x480x24 -noreset &
sleep 2

start_worker 18081 "$tmp/server.log"
RUST_LOG=info,stream=debug DISPLAY=:97 $bin/peer-poc --server ws://127.0.0.1:18081/ws --key-file "$tmp/host.key" \
    host --source pattern >"$tmp/host.log" 2>&1 &
for _ in $(seq 100); do
    password=$(sed -n 's/^Password: //p' "$tmp/host.log")
    [ -n "$password" ] && break
    sleep 0.1
done
id=$(sed -n 's/^Registered as //p' "$tmp/host.log")

# Viewer copied ~100 KB of text before connecting (larger than one 16 KB data channel chunk).
head -c 75000 /dev/urandom | base64 -w 0 >"$tmp/long.txt"
DISPLAY=:98 xclip -selection clipboard -i "$tmp/long.txt"

RUST_LOG=info,stream=debug DISPLAY=:98 $bin/peer-poc --server ws://127.0.0.1:18081/ws \
    connect "$id" -p "$password" --clipboard --seconds 60 >"$tmp/viewer.log" 2>&1 &
for _ in $(seq 300); do
    grep -q "^Host streaming" "$tmp/viewer.log" && break
    sleep 0.1
done
grep -q "^Host streaming" "$tmp/viewer.log" || { echo "FAIL: session not up"; exit 1; }
sleep 2

echo "== viewer → host: text copied before connecting"
DISPLAY=:97 xclip -o -selection clipboard >"$tmp/got.txt"
cmp "$tmp/long.txt" "$tmp/got.txt"

echo "== viewer → host: files and a folder"
mkdir -p "$tmp/src/cartella/sotto"
echo "ciao dal viewer" >"$tmp/src/nota.txt"
head -c 300000 /dev/urandom >"$tmp/src/cartella/sotto/dati.bin"
printf 'file://%s\nfile://%s\n' "$tmp/src/nota.txt" "$tmp/src/cartella" |
    DISPLAY=:98 xclip -selection clipboard -t text/uri-list -i
sleep 4
DISPLAY=:97 xclip -o -selection clipboard -t text/uri-list | tr -d '\r' >"$tmp/uris.txt"
cat "$tmp/uris.txt"
note=$(grep 'nota.txt$' "$tmp/uris.txt" | sed 's|^file://||')
folder=$(grep 'cartella$' "$tmp/uris.txt" | sed 's|^file://||')
case "$note" in "$tmp/src/"*) echo "FAIL: host got the viewer's own paths"; exit 1 ;; esac
cmp "$tmp/src/nota.txt" "$note"
cmp "$tmp/src/cartella/sotto/dati.bin" "$folder/sotto/dati.bin"

echo "== host → viewer: text"
echo -n "copiato sull'host" | DISPLAY=:97 xclip -selection clipboard -i
sleep 3
test "$(DISPLAY=:98 xclip -o -selection clipboard)" = "copiato sull'host"

grep -E "^Clipboard" "$tmp/viewer.log" "$tmp/host.log" || true
echo "clipboard e2e ok"
