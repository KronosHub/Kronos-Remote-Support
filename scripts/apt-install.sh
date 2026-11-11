#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner without hanging: the package mirrors sometimes stop
# answering and apt-get then waits for hours. Each attempt is time-limited and retried; after a
# failure only Ubuntu's main archive is kept. (release.yml has the same steps inline.)
# Usage: scripts/apt-install.sh <package>...
set -uo pipefail

opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
for attempt in 1 2 3; do
  if timeout 300 sudo apt-get "${opts[@]}" update &&
     timeout 600 sudo apt-get "${opts[@]}" install -y "$@"; then
    exit 0
  fi
  echo "::warning::apt-get attempt $attempt failed or timed out"
  sudo find /etc/apt/sources.list.d -type f ! -name ubuntu.sources -delete
  sudo sed -i 's#[a-z]*\.archive\.ubuntu\.com#archive.ubuntu.com#g' \
    /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null
  sleep 15
done
echo "::error::apt-get failed after 3 attempts"
exit 1
