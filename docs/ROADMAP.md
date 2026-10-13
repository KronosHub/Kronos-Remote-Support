# Roadmap

Legenda: [x] fatto · [~] in corso · [ ] da fare

Ogni fase produce qualcosa di eseguibile e verificabile.

## Fase 0 — Setup
- [x] Repo git + workspace Cargo
- [x] Scelte aperte risolte (libreria WebRTC, framework UI, decodifica lato viewer) — ADR 0004–0008
- [x] CI base (`.github/workflows/ci.yml`), repository GitHub

## Fase 1 — Cattura e encoding locale (PoC)
- [x] `capture`: cattura schermo su piattaforma di sviluppo (macOS), misura FPS
- [x] Default su display principale
- [x] `codec`: encoding H.264, salvataggio su file `.h264`, verifica con ffplay
- [x] Porting cattura su Windows (DXGI) e Linux X11 (ADR 0019)
- [ ] Linux Wayland (portal + PipeWire)
- [ ] Encoder hardware (VideoToolbox) se OpenH264 non regge Retina/60 fps

## Fase 2 — Signaling + connessione WebRTC
- [x] Signaling server: registrazione ID, inoltro SDP/ICE
- [x] Due istanze CLI si connettono (stessa macchina), data channel "ping/pong"
- [x] Deploy su VM Oracle (nativo + systemd + Caddy, `deploy/`)
- [ ] Test tra due macchine diverse via server remoto
- [x] App desktop con GUI (Tauri + Svelte), icona, tray, istanza singola (ADR 0013)
- [x] Installer per 3 OS via GitHub Actions (ADR 0014) — v0.1.0 in release bozza

## Fase 3 — Streaming video
- [x] Host invia video track, viewer decodifica e mostra in finestra Tauri (ADR 0015)
- [x] Richieste keyframe, gestione perdite, schermo intero, disconnessione, banner "schermo condiviso" + Termina
- [x] Fix RTT: esclusione interfacce VPN da ICE (ADR 0016)
- [ ] Test tra due macchine reali (Mac ↔ Windows) con la nuova versione
- [ ] Verificare WebCodecs su Linux (WebKitGTK); fallback se assente
- [x] Robustezza: buffer di riordino NACK, keyframe limitati, guardia decoder (ADR 0022)
- [ ] Latenza glass-to-glass, adattamento bitrate alla rete (TWCC), pacing keyframe

## Fase 4 — Controllo remoto
- [x] Eventi mouse/tastiera via data channel (ADR 0018)
- [x] Iniezione input sulle 3 piattaforme, tasti per posizione fisica (Windows/Linux X11 verificati in CI)
- [x] Coordinate normalizzate sul display principale
- [ ] Test reale Mac ↔ Windows ↔ Linux
- [x] Multi-monitor: scelta dello schermo (ADR 0020)
- [ ] Traduzione modificatori ⌘/Ctrl, scorciatoie di sistema

## Fase 5 — NAT traversal
- [x] Credenziali TURN temporanee nel signaling, consegnate a ogni sessione (ADR 0024)
- [x] Tipo di connessione visibile nell'app (ADR 0025)
- [x] coturn sulla VM (4.18, Oracle EPEL)
- [x] Test `--relay-only` (30 fps, RTT ~44 ms)
- [ ] Test da reti diverse (es. hotspot telefono); apertura più rapida in relay; TURN TLS 443

## Fase 6 — Sicurezza
- [x] Password + PAKE legato alle fingerprint DTLS (ADR 0012)
- [x] Rate limit, conferma lato host
- [x] Indicatore sessione attiva + termina sessione
- [ ] Firma stabile del codice (oggi ogni build macOS richiede di riconcedere il permesso schermo)
- [x] Password personale per accesso non presidiato, I miei dispositivi, avvio all'accesso (ADR 0021)
- [ ] Device key sostituita da coppia di chiavi + challenge (ADR 0010)

## Fase 7 — Funzioni extra
- [ ] Clipboard condivisa
- [ ] Trasferimento file
- [ ] Selezione monitor
- [ ] Chat

## Fase 8 — Distribuzione
- [ ] Installer (MSI, DMG firmato, AppImage/deb)
- [ ] Accesso non presidiato (servizio di sistema) — opzionale
- [ ] Wayland
