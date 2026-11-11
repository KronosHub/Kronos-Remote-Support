#!/usr/bin/env bash
# Installs Ubuntu packages on a CI runner without hanging: package mirrors sometimes stop
# answering and apt-get then waits for hours. Each attempt is time-limited; later attempts
# use other mirrors. (release.yml has the same steps inline.)
# Usage: scripts/apt-install.sh <package>...
pkgs=("$@")
set +e
. /etc/os-release
opts=(-o Acquire::Retries=1 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)
# 1: the runner's own sources; then mirrors outside ubuntu.com (the runners' DNS has failed
# to resolve the ubuntu.com hosts while other names worked).
mirrors=("" mirrors.edge.kernel.org/ubuntu mirror.math.princeton.edu/pub/ubuntu)
for attempt in 1 2 3; do
  m=${mirrors[$((attempt - 1))]}
  if [ -n "$m" ]; then
    echo "Using mirror $m"
    sudo rm -f /etc/apt/sources.list.d/*
    printf 'deb http://%s %s main restricted universe multiverse\n' \
      "$m" "$VERSION_CODENAME" "$m" "$VERSION_CODENAME-updates" "$m" "$VERSION_CODENAME-security" |
      sudo tee /etc/apt/sources.list >/dev/null
  fi
  timeout 120 sudo apt-get "${opts[@]}" update &&
    timeout 600 sudo apt-get "${opts[@]}" install -y "${pkgs[@]}" && exit 0
  echo "::warning::apt-get attempt $attempt failed or timed out"
done
echo "::error::apt-get failed after 3 attempts"
exit 1
