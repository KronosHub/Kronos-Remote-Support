# 0024 — Relay TURN (coturn) sulla VM

**Data:** 2026-10-08 · **Stato:** accettata (deploy sulla VM in attesa di conferma utente)

## Decisione
- **coturn** (Oracle EPEL) sulla VM: porta 3478 UDP/TCP (STUN + TURN), relay UDP 49160–49200, `external-ip`
  pubblico/privato (la VM OCI è dietro NAT 1:1). Fa anche da STUN: non si usa più lo STUN di Google.
- **Credenziali temporanee** (TURN REST API, `use-auth-secret`): il signaling firma
  `username = "<scadenza>:<id>"`, `password = base64(HMAC-SHA1(segreto, username))`, validità 12 h.
  Il segreto è generato sulla VM (`/etc/desktop-remoto/turn-secret`, 0600) e passato al signaling come
  credenziale systemd (`LoadCredential`); non esce mai dalla VM né entra nelle app.
- Le credenziali sono inviate **a ogni sessione** (`SessionCreated` / `IncomingSession` con `ice_servers`),
  così un dispositivo online da giorni (accesso non presidiato) non usa credenziali scadute.
- Sicurezza coturn: niente CLI, niente TLS/DTLS di coturn (il contenuto è già DTLS end-to-end tra i peer),
  `denied-peer-ip` per reti private, loopback, link-local (metadati cloud), CGNAT; quote per utente e totali.
- Test: `peer-poc --relay-only` (policy ICE "relay") forza il percorso TURN.

## Conseguenze
- Porte da aprire anche nella Security List OCI: UDP 3478, TCP 3478, UDP 49160–49200.
- Banda: il relay passa dalla VM (free tier ~50 Mbit/s): adatto a poche sessioni contemporanee.
- TURN su TCP 443/TLS (reti che permettono solo HTTPS) non coperto: possibile evoluzione.
