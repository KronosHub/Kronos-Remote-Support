# 0018 — Controllo remoto: eventi input e iniezione sull'host

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
- **Trasporto**: `ControlMsg::Input { event }` sul canale `control` (affidabile, ordinato).
  `InputEvent` = `mouse_move {x,y}` normalizzati 0–1 sull'immagine condivisa, `mouse_button`, `wheel {dx,dy}` in tacche,
  `key {code, down}` con `code` = `KeyboardEvent.code` W3C (posizione fisica).
- **Tasti per posizione fisica**: crate `input`, tabella `code` → kVK macOS / scancode Windows (E0 = esteso) /
  evdev Linux (X11 = evdev+8). Il layout dell'host decide il carattere, come una tastiera vera.
  Auto-repeat: dalla 1.2.3 il viewer inoltra le ripetizioni come nuovi "key down" (su macOS e Windows i tasti
  simulati non si ripetono da soli: tenendo premuta una freccia il cursore si spostava di un solo carattere);
  l'host Linux le scarta perché X ripete da sé i tasti premuti.
- **Iniezione**: thread dedicato (`InputSink`). `enigo` 0.6 per mouse ovunque e tasti su macOS/X11; tasti Windows
  con `SendInput` + `KEYEVENTF_SCANCODE` (gestione estesi più chiara che in `enigo::raw`).
- **Sicurezza**: alla fine della sessione si rilasciano tasti e pulsanti ancora premuti; la UI rilascia i tasti se
  perde il focus. Il popup di conferma dice "vedere e controllare".
- **Permessi**: macOS richiede *Accessibilità* (prompt al primo uso). Se manca, l'host invia `InputUnavailable`:
  il viewer mostra "Solo visione" e la visione continua.
- **Coordinate**: il viewer normalizza sull'immagine; l'host scala sulle dimensioni del display principale
  (`enigo::main_display`). Per coerenza la cattura macOS ora usa il display principale (origine 0,0).

## Limiti noti
- Scorciatoie intercettate dal sistema del viewer (⌘Tab, Alt+Tab, Ctrl+Alt+Canc) non arrivano all'host.
- Nessuna traduzione modificatori tra sistemi (⌘ del Mac arriva come tasto Windows/Super).
- Linux Wayland: non supportato (servirà portal RemoteDesktop / libei).
- Windows: finestre elevate (amministratore) e desktop sicuro (UAC) non controllabili da un processo non elevato.
