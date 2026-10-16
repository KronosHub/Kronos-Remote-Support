# 0052 — Quick Support con ID fisso e password personale; cartelle dei dispositivi

**Data:** 2026-10-16 · **Stato:** accettata (rivede in parte la 0031)

## Richiesta
1. Kronos Quick Support con un ID che non cambia e la possibilità di una password per l'accesso non presidiato.
2. In "I miei dispositivi", gruppi di dispositivi ad albero con cartelle che si chiudono.

## Decisione
### Quick Support
- Usa una cartella dati come l'app completa, ma la sua (`tech.kronos.quicksupport`: `%APPDATA%` su Windows,
  `~/Library/Application Support` su macOS, `~/.local/share` su Linux). Ci tiene `device.key` (quindi lo stesso
  ID a ogni avvio), il segreto della password personale e le preferenze (nome). L'ID è diverso da quello dell'app
  completa sullo stesso computer: sono due dispositivi distinti.
- La password personale si imposta come nell'app completa (stesso segreto derivato, stessa verifica SPAKE2, stesso
  limite ai tentativi). Il backend non la rifiuta più in modalità Quick.
- Restano esclusi: avvio automatico, icona nella barra, aggiornamenti, connessione ad altri, dispositivi salvati.
  Chiudere la finestra chiude il programma: l'accesso non presidiato vale solo mentre Quick Support è aperto, e
  non resta nulla in ascolto dopo.
- I log restano solo sulla console.
- Viene meno la promessa "nulla resta sul computer" della 0031: restano i piccoli file nella cartella dati.

### Cartelle
- `groups.json` accanto a `devices.json`: `{id, name, parent?, collapsed?}`; il dispositivo salvato ha `group?`.
  I file delle versioni precedenti si leggono senza modifiche.
- Le cartelle sono sempre un albero: non si sposta una cartella dentro sé stessa o una sua sottocartella; un file
  danneggiato (genitore mancante o ciclo) viene mostrato con la cartella al primo livello.
- Eliminare una cartella sposta dispositivi e sottocartelle nella cartella che la conteneva: nessun dispositivo
  si perde. Salvare di nuovo un dispositivo (nuova password) lo lascia nella sua cartella.
- Lo stato aperto/chiuso è salvato. Spostamento con un menu (non trascinamento: su Windows il trascinamento dei
  file di Tauri intercetta quello HTML).

## Verifica
Test di `store.rs` (cartelle annidate, spostamenti vietati, eliminazione, file con ciclo, file vecchi); interfaccia
controllata con dati finti in Chrome headless. Quick Support avviato due volte su macOS: stesso ID (974 263 692). Da
provare sul campo: connessione con password personale a Quick Support.
