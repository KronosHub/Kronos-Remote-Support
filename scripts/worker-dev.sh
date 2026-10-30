# Sourced by the end-to-end scripts: starts the real signaling Worker locally (wrangler dev,
# deploy/cloudflare) on port $1 in the background and waits until it answers.
# Needs Node; installs the Worker's dependencies the first time.
start_worker() {
    local port=$1 log=$2
    if [ ! -d deploy/cloudflare/node_modules ]; then
        (cd deploy/cloudflare && npm ci --silent)
    fi
    (cd deploy/cloudflare && exec npx wrangler dev --ip 127.0.0.1 --port "$port" \
        --persist-to "$(dirname "$log")/worker-state" --log-level warn) >"$log" 2>&1 &
    for _ in $(seq 120); do
        curl -fs "http://127.0.0.1:$port/health" >/dev/null 2>&1 && return 0
        sleep 0.5
    done
    echo "signaling Worker not ready"; cat "$log"; return 1
}
