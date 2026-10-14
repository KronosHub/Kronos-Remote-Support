# 0043 — Computer controllato sempre sveglio durante la sessione

**Data:** 2026-10-14 · **Stato:** accettata

## Problema
Un Mac controllato a distanza è andato in blocco schermo mentre lo si usava: l'input simulato non sempre conta
come attività dell'utente, quindi scattavano lo stop del monitor (o il salvaschermo) e la richiesta di password.

## Decisione
`stream::awake::KeepAwake`, tenuto per tutta la sessione host (`serve_host`):
- **macOS**: asserzione IOKit `PreventUserIdleDisplaySleep` per la durata della sessione e
  `IOPMAssertionDeclareUserActivity` a ogni input remoto (al massimo ogni 10 s), che azzera anche il timer del
  salvaschermo. FFI diretta a IOKit/CoreFoundation, senza nuove dipendenze.
- **Windows**: `SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED)` su un thread
  dedicato che vive quanto la sessione (lo stato appartiene al thread).
- **Linux**: nulla per ora.

## Conseguenze
- Finita la sessione il computer torna a seguire le sue impostazioni di risparmio energetico.
- Una sessione lasciata aperta tiene acceso il monitor del computer controllato (come gli altri strumenti di
  assistenza remota).
