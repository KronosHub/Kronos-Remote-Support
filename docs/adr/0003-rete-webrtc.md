# 0003 — Rete: WebRTC + signaling e TURN self-hosted

**Data:** 2026-10-07 · **Stato:** accettata

## Contesto
Serve connettere peer dietro NAT con bassa latenza e cifratura.

## Decisione
WebRTC (ICE/STUN/TURN, DTLS-SRTP). Signaling server proprio in Rust (axum + WebSocket).
TURN con coturn self-hosted.

## Conseguenze
- NAT traversal e cifratura standard inclusi.
- Libreria WebRTC Rust ancora da scegliere (domanda aperta in STATO.md).
- Il signaling può teoricamente fare MITM sulle fingerprint DTLS: mitigato con PAKE (vedi ARCHITETTURA.md).
