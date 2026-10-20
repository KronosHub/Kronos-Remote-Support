#!/usr/bin/env bash
# GitHub Actions, Windows: builds the minimal FFmpeg (build.sh) from the runner's MSYS2 shell,
# started from a cmd where vcvars64.bat set up MSVC. Usage: ci-windows.sh <output dir>
set -euo pipefail
pacman -Sy --noconfirm --needed make nasm diffutils patch
# MSYS2's own link.exe would shadow MSVC's linker for configure.
rm -f /usr/bin/link.exe
tools/ffmpeg/build.sh x86_64-pc-windows-msvc "$(cygpath -u "$1")"
