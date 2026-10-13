# 0012 — Password di sessione, SPAKE2 e conferma dell'host

**Data:** 2026-10-07 · **Stato:** accettata (anticipa parte della Fase 6)

## Requisito utente
Oltre all'ID serve una password alfanumerica minuscola; l'host deve confermare con un popup.

## Decisione
- **Password**: 8 caratteri da `abcdefghijkmnpqrstuvwxyz23456789` (esclusi l/o/0/1), ~40 bit.
  Generata all'avvio, non salvata, rigenerabile dall'utente ("Nuova"). Input normalizzato (minuscolo, senza spazi).
- **Verifica peer-to-peer** sul data channel `control` già cifrato (DTLS), crate `crates/auth`:
  SPAKE2 simmetrico (crate `spake2` 0.4, Ed25519) + conferma chiave con HMAC-SHA256 su trascrizione che
  include i fingerprint DTLS di entrambi (letti dagli SDP negoziati).
  - la password non lascia mai il dispositivo, nemmeno cifrata;
  - niente brute force offline da chi osserva (server incluso);
  - un MITM che sostituisce i certificati in signaling fa fallire la conferma anche con password giusta.
- **Messaggi** (`transport::handshake`): `spake` ↔ `spake`, `proof` →, ← `accepted{mac}` | `denied{reason}`.
  Il viewer verifica anche la prova dell'host (autenticazione reciproca).
- **Conferma host**: il popup appare solo *dopo* password corretta; timeout 30 s = rifiuto.
- **Anti brute force**: dopo 5 password errate in 10 minuti l'host rifiuta subito (`troppi tentativi`).

## Conseguenze
- La protezione MITM vale solo se la libreria WebRTC verifica il certificato DTLS contro il fingerprint SDP (comportamento standard).
- Password rigenerata a ogni avvio: per accesso non presidiato servirà una password permanente (futuro).
