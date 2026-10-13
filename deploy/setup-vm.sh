#!/usr/bin/env bash
# VM setup (Oracle Linux 9): Caddy binary + user, coturn (TURN relay), firewall. Idempotent.
# Runs ON the VM via deploy.sh. Idempotent.
set -euo pipefail
CADDY_VERSION="${CADDY_VERSION:-2.11.7}"
: "${PUBLIC_IP:?PUBLIC_IP must be set (passed by deploy.sh)}"

if [ "$(/usr/local/bin/caddy version 2>/dev/null | awk '{print $1}')" != "v$CADDY_VERSION" ]; then
    tmp=$(mktemp -d)
    base="https://github.com/caddyserver/caddy/releases/download/v$CADDY_VERSION"
    curl -fsSL -o "$tmp/caddy.tar.gz" "$base/caddy_${CADDY_VERSION}_linux_amd64.tar.gz"
    curl -fsSL -o "$tmp/checksums.txt" "$base/caddy_${CADDY_VERSION}_checksums.txt"
    (cd "$tmp" && grep "caddy_${CADDY_VERSION}_linux_amd64.tar.gz" checksums.txt \
        | sed 's/caddy_.*_linux_amd64.tar.gz/caddy.tar.gz/' | sha512sum -c -)
    tar -xzf "$tmp/caddy.tar.gz" -C "$tmp" caddy
    sudo install -m 0755 "$tmp/caddy" /usr/local/bin/caddy
    rm -rf "$tmp"
fi

getent group caddy >/dev/null || sudo groupadd --system caddy
getent passwd caddy >/dev/null || sudo useradd --system --gid caddy --home-dir /var/lib/caddy \
    --create-home --shell /sbin/nologin caddy
sudo mkdir -p /etc/caddy

sudo firewall-cmd --quiet --permanent --add-service=http --add-service=https

# --- TURN relay (coturn from Oracle EPEL) -------------------------------------------
if ! rpm -q coturn >/dev/null; then
    rpm -q oracle-epel-release-el9 >/dev/null || sudo dnf install -y -q oracle-epel-release-el9
    sudo dnf install -y -q --enablerepo=ol9_developer_EPEL coturn
fi
# Shared secret with the signaling server: generated here once, never leaves the VM.
sudo mkdir -p /etc/desktop-remoto && sudo chmod 700 /etc/desktop-remoto
if ! sudo test -s /etc/desktop-remoto/turn-secret; then
    openssl rand -hex 32 | sudo tee /etc/desktop-remoto/turn-secret >/dev/null
    sudo chmod 600 /etc/desktop-remoto/turn-secret
fi
private_ip=$(hostname -I | awk '{print $1}')
secret=$(sudo cat /etc/desktop-remoto/turn-secret)
sed -e "s/@PUBLIC_IP@/$PUBLIC_IP/" -e "s/@PRIVATE_IP@/$private_ip/" -e "s/@TURN_SECRET@/$secret/" \
    "$(dirname "$0")/turnserver.conf" | sudo tee /etc/coturn/turnserver.conf >/dev/null
sudo chown root:coturn /etc/coturn/turnserver.conf && sudo chmod 640 /etc/coturn/turnserver.conf
sudo firewall-cmd --quiet --permanent --add-port=3478/udp --add-port=3478/tcp --add-port=49160-49200/udp
# The distro unit only restarts on abort: bring coturn back after any failure, and only
# once the network (and the private IP it binds to) is up.
sudo mkdir -p /etc/systemd/system/coturn.service.d
printf '[Unit]\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nRestart=on-failure\nRestartSec=2\n' \
    | sudo tee /etc/systemd/system/coturn.service.d/override.conf >/dev/null
sudo systemctl daemon-reload
sudo systemctl enable --quiet coturn
sudo systemctl restart coturn

sudo firewall-cmd --quiet --reload
echo "setup ok: $(/usr/local/bin/caddy version), coturn $(rpm -q --qf '%{VERSION}' coturn)"
