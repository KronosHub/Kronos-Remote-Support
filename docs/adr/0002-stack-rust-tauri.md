# 0002 — Stack: Rust + Tauri

**Data:** 2026-10-07 · **Stato:** accettata

## Contesto
Servono accesso a basso livello (cattura schermo, codec, input) e UI multipiattaforma.

## Decisione
Core in Rust (workspace di crate), UI con Tauri (frontend web).
Alternative scartate: C++/Qt (più lento da sviluppare), Electron (pesante, limitato a basso livello),
Flutter+Rust (utile solo se mobile, fuori scope).

## Conseguenze
- Binari piccoli, prestazioni native, sicurezza della memoria.
- Riferimento utile: RustDesk (open source, stessa famiglia di stack).
