# 0004 — Libreria WebRTC: webrtc-rs

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
Usare `webrtc-rs` (crate `webrtc`), implementazione pure Rust completa (ICE, DTLS, SRTP, data channel).

## Alternative
- `str0m`: sans-IO, sviluppo attivo, ma più lavoro di integrazione.
- binding `libwebrtc`: potente, build pesante e complessa.

## Conseguenze
- Avvio rapido. Rischio: manutenzione rallentata. Isolare l'uso dentro il crate `transport`
  così da poter passare a `str0m` senza toccare il resto.

## Aggiornamento 2026-10-07
Versione usata: `webrtc` 0.21. Dalla 0.20 il progetto è stato riscritto: wrapper async sopra il core sans-IO
`rtc` (stesso approccio di `str0m`), con runtime tokio/smol selezionabile. API diversa dalle versioni ≤0.12
(es. `PeerConnectionEventHandler` invece delle callback `on_*`). Il rischio "manutenzione rallentata" risulta ridotto.
