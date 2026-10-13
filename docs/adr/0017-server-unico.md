# 0017 — Server di signaling unico, non configurabile dall'app

**Data:** 2026-10-07 · **Stato:** accettata (richiesta utente)

## Decisione
L'app usa solo il server sulla VM, `wss://<ip-con-trattini>.sslip.io/ws` (costante `SERVER` in `apps/desktop/src-tauri/src/service.rs`).
Dalla v0.12.0 la costante punta al Worker Cloudflare `wss://kronos-signaling.kronos-signaling.workers.dev/ws` (ADR 0035).
Rimossi dall'interfaccia indirizzo del server e "Cambia server"; rimossi comando `set_server` e `settings.json`.

## Conseguenze
Per cambiare server serve una nuova build. Da riconsiderare se servirà un server per cliente/organizzazione.
