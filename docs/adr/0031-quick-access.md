# 0031 — Kronos Quick Support (accesso rapido senza installazione)

**Data:** 2026-10-08 · **Stato:** accettata

## Richiesta
Una versione del client che non richiede installazione e serve solo ad avviare la condivisione (non a connettersi ad
altri). Scelte dell'utente: Windows + macOS + Linux; solo password temporanea + conferma.

## Decisione
- **Stesso codice**, feature Cargo `quick-access` (`QUICK` in `lib.rs`) + `tauri.quick.conf.json` (merge):
  nome "Kronos Quick Support", identificativo `tech.kronos.quicksupport` (dati e permessi macOS separati dall'app
  completa), binario `kronos-quick-support`, finestra più stretta, web view `incognito`, niente artefatti updater.
- **Comportamento Quick**: ID nuovo a ogni avvio (chiave dispositivo solo in memoria), `Store` senza cartella (nulla
  letto/scritto), log solo su console; niente tray, avvio automatico, updater, password personale, dispositivi salvati;
  chiudere la finestra chiude l'app e la condivisione. Il backend rifiuta `Connect` (non solo la UI lo nasconde) e
  `set_personal_password`; l'host rifiuta i tentativi con password personale ("accesso non presidiato non attivo").
- **UI**: comando `app_mode`; in modalità quick solo "Consenti il controllo" (ID, password, permessi macOS) e Attività.
- **Pacchetti** (job in `release.yml` del repo pubblico, dopo l'app completa): Windows `.exe` singolo (`--no-bundle`,
  serve WebView2), macOS `.app` firmata con l'identità stabile e zippata (`ditto`), Linux AppImage.

## Verifica (macOS)
Bundle 13 MB firmato (`identifier "tech.kronos.quicksupport"`, stesso certificato); registrazione al server; tentativo
con password personale rifiutato; UI ridotta come previsto; dopo la chiusura nessun file in `~/Library` (solo cartelle
temporanee di WebKit in `$TMPDIR`).

## Limiti noti
- Windows: resta la cartella dati di WebView2 in `%LOCALAPPDATA%\tech.kronos.quicksupport`.
- Senza firma Authenticode gli antivirus (es. WithSecure, segnalato dall'utente sulla 0.8.0) e SmartScreen bloccano o
  avvisano: per l'uso con i clienti serve firmare gli eseguibili Windows (decisione/costo dell'utente).
