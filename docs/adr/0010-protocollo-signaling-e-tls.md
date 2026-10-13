# 0010 — Protocollo di signaling e TLS senza dominio

**Data:** 2026-10-07 · **Stato:** accettata

## Protocollo
- WebSocket, messaggi JSON (`crates/signaling-proto`), discriminati da campo `type`.
- Client → server: `register{device_key}`, `connect{target}`, `signal{session,payload}`, `leave{session}`.
- Server → client: `registered{id,ice_servers}`, `session_created`, `incoming_session`, `signal`, `session_closed`, `error{code}`.
- `payload` = `offer{sdp}` | `answer{sdp}` | `candidate{...}` (trickle ICE). Il server lo inoltra senza interpretarlo.
- Chi chiama `connect` (viewer) è l'offerer e crea il data channel `control`.
- ID: 9 cifre casuali, legate a SHA-256 della `device_key` (la chiave in chiaro non viene salvata), persistite in `ids.json`.
- Il server comunica gli ICE server ai client: in Fase 5 servirà per le credenziali TURN temporanee.
- Protezioni minime: messaggi ≤ 64 KiB, `register` entro 10 s, max 10 `connect` al minuto per connessione,
  relay ammesso solo tra le due parti della sessione.

## TLS
Server su HTTP in chiaro (solo localhost) dietro **Caddy**, che termina TLS con certificato Let's Encrypt automatico.
Senza dominio: hostname `<ip-con-trattini>.sslip.io` (DNS pubblico gratuito che risolve all'IP contenuto nel nome).

## Limiti noti (da risolvere)
- Chi conosce la `device_key` può impersonare quel dispositivo: in futuro sostituirla con coppia di chiavi + challenge.
- Nessuna conferma lato host: chiunque conosca l'ID apre una sessione WebRTC (ma senza password non otterrà
  nulla una volta implementato il PAKE, Fase 6).
- Rate limit per connessione, non per IP.

Modalità di deploy: vedi ADR 0011 (nativo + systemd).
