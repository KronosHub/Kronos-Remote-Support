# 0005 — Viewer: decodifica video in Rust

**Data:** 2026-10-07 · **Stato:** superata in parte dall'ADR 0015 (decodifica con WebCodecs nella UI)

## Contesto
La webview di Tauri su Linux (WebKitGTK) ha supporto WebRTC scarso/disabilitato.

## Decisione
Il viewer riceve la traccia WebRTC nel core Rust, decodifica H.264 (OpenH264 o decoder hardware)
e passa i frame alla UI per il rendering (canvas/WebGL, oppure superficie wgpu nativa se serve performance).

## Conseguenze
- Comportamento identico su Windows/macOS/Linux.
- Da progettare: canale efficiente core→UI per i frame (Fase 3).
