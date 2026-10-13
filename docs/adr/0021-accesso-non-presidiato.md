# 0021 — Accesso non presidiato con password personale

**Data:** 2026-10-08 · **Stato:** accettata (richiesta utente)

## Requisito
Controllare i propri dispositivi senza conferma sull'host.

## Decisione
- **Password personale** impostata sull'host (min. 8 caratteri, maiuscole/minuscole distinte, no spazi ai bordi),
  in aggiunta alla password temporanea. Valida finché non viene cambiata o disattivata.
- Non si salva mai la password: entrambi i lati usano `derive_personal_secret` = PBKDF2-HMAC-SHA256
  (100.000 iterazioni) con sale `desktop-remoto/personal/v1/<id host>`. Il segreto vale solo per quell'host.
- Handshake: il primo messaggio `spake` del viewer porta `personal: true`; l'host esegue SPAKE2 con il segreto
  personale e, se la prova è valida, accetta **senza popup** (`Access::Unattended`). Senza password personale
  impostata l'host rifiuta ("accesso non presidiato non attivo"). Compatibile con peer vecchi (campo opzionale).
- Restano: verifica peer-to-peer (il server non vede nulla), limite tentativi (5 errori in 10 minuti),
  banner "schermo condiviso" + Termina sull'host, finestra mostrata e voce nel registro attività.
- **Viewer**: casella "Password personale (senza conferma)" + "Salva come"; elenco **I miei dispositivi**
  (`devices.json`, solo id/nome/segreto derivato) con Connetti/Rimuovi.
- File `personal.json` e `devices.json` nella cartella dati dell'app, scritti in modo atomico, permessi 0600 su Unix.
- **Avvia all'accesso** (`tauri-plugin-autostart`, LaunchAgent su macOS): l'app parte nascosta (`--hidden`)
  nella barra di sistema, così il dispositivo resta raggiungibile dopo un riavvio.

## Scartato
- Elenco di "dispositivi autorizzati" per ID: l'ID di chi si collega non è autenticato (lo dichiara il server),
  quindi non è una prova d'identità. Servirebbero chiavi per dispositivo (futuro, vedi ADR 0010).

## Limiti
- Il segreto derivato è equivalente alla password per quell'host: chi copia `devices.json` può connettersi.
  Evoluzione: portachiavi di sistema (Keychain / Credential Manager / Secret Service).
- macOS: schermata di login e schermo bloccato non sono controllabili senza un servizio di sistema.
