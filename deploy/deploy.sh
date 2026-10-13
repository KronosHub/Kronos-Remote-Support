#!/usr/bin/env bash
# Build the signaling server for Linux on this Mac and install/update it on the VM.
#   VM_HOST=<vm-public-ip> VM_KEY=~/key.pem ./deploy/deploy.sh
set -euo pipefail
cd "$(dirname "$0")/.."

: "${VM_HOST:?set VM_HOST (public IP)}"
: "${VM_KEY:?set VM_KEY (ssh private key)}"
VM_USER="${VM_USER:-opc}"
SIGNALING_HOST="${SIGNALING_HOST:-${VM_HOST//./-}.sslip.io}"
ssh_vm() { ssh -i "$VM_KEY" -o BatchMode=yes "$VM_USER@$VM_HOST" "$@"; }

cargo zigbuild --release -p signaling --target x86_64-unknown-linux-musl
bin=target/x86_64-unknown-linux-musl/release/signaling

stage=$(ssh_vm mktemp -d)
scp -q -i "$VM_KEY" "$bin" deploy/signaling.service deploy/caddy.service deploy/Caddyfile \
    deploy/setup-vm.sh deploy/turnserver.conf "$VM_USER@$VM_HOST:$stage/"

ssh_vm "set -euo pipefail
    PUBLIC_IP=$VM_HOST bash $stage/setup-vm.sh
    # install (not mv) so SELinux gives the files their default labels.
    sudo install -m 0755 $stage/signaling /usr/local/bin/signaling
    sed 's/@PUBLIC_IP@/$VM_HOST/g' $stage/signaling.service | sudo tee /etc/systemd/system/signaling.service >/dev/null
    sudo install -m 0644 $stage/caddy.service /etc/systemd/system/
    sed 's/{\\\$SIGNALING_HOST}/$SIGNALING_HOST/' $stage/Caddyfile | sudo tee /etc/caddy/Caddyfile >/dev/null
    sudo restorecon -R /usr/local/bin /etc/systemd/system /etc/caddy
    rm -rf $stage
    sudo systemctl daemon-reload
    sudo systemctl enable --quiet signaling caddy
    sudo systemctl restart signaling
    sudo systemctl reload-or-restart caddy
    systemctl is-active signaling caddy coturn"

echo "deployed: wss://$SIGNALING_HOST/ws  (check: curl https://$SIGNALING_HOST/health)"
