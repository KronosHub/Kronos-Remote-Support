# 0014 — Build e distribuzione con GitHub Actions

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
- macOS: build locale possibile (`npx tauri build` in `apps/desktop`) → `.app` + `.dmg`.
- Windows: **niente cross-compilazione dal Mac** (scelta utente: evitare di accettare la licenza SDK Microsoft via xwin);
  build su runner `windows-latest`.
- Linux: build su runner `ubuntu-22.04` (WebKitGTK 4.1).
- Workflow `.github/workflows/release.yml` (tauri-action): al push di un tag `v*` o manuale, compila macOS arm64 + x86_64,
  Windows (MSI + NSIS), Linux (deb, rpm, AppImage) e li allega a una release GitHub in bozza.
- Workflow `ci.yml`: fmt, clippy `-D warnings`, test, e2e locale a ogni push.

## Conseguenze
- Serve un repository GitHub (privato va bene).
- Installer non firmati: avvisi Gatekeeper (macOS) e SmartScreen (Windows). Firma/notarizzazione: futuro.
