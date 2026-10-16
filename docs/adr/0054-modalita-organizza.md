# 0054 — Modalità Organizza per spostare i dispositivi nelle cartelle

**Data:** 2026-10-16 · **Stato:** accettata (v1.8.2), rivede lo spostamento della 0052

## Contesto
Nella 1.8.0 ogni dispositivo aveva un menu a tendina che lo spostava appena si cambiava la scelta. L'utente ha
segnalato il rischio di spostare un dispositivo per errore. Proposte valutate: modalità Organizza, menu ⋯ con
finestra Sposta, trascinamento con Annulla. Scelta dell'utente: modalità Organizza.

## Decisione
- Nella vista normale non c'è nulla che sposti i dispositivi o modifichi le cartelle: restano Connetti, Rimuovi
  (con conferma) e l'apertura/chiusura delle cartelle.
- **Organizza** mostra le caselle di selezione (una sulla cartella seleziona tutti i dispositivi che contiene,
  sottocartelle comprese), i pulsanti delle cartelle (+ Cartella, Modifica, Elimina) e in alto Nuova cartella,
  **Sposta in… (n)** e **Fine**; Connetti e Rimuovi sono nascosti.
- **Sposta in…** apre una finestra con l'elenco delle cartelle (scelta con pulsanti radio) e il tasto Sposta: lo
  spostamento richiede sempre una conferma esplicita e può riguardare più dispositivi insieme.
- Dopo ogni spostamento una barra "… spostati in X — Annulla" resta per 10 secondi e riporta ogni dispositivo nella
  cartella di prima.
- Nessun cambiamento nel backend: si usa `move_device` una volta per dispositivo.

## Conseguenze
- Spostare un solo dispositivo richiede più clic di prima (Organizza, spunta, Sposta in…, Sposta).
- Il trascinamento resta escluso per il motivo della 0052 (su Windows Tauri intercetta il trascinamento HTML).

## Verifica
Interfaccia controllata con dati finti in Chrome headless: selezione di una cartella annidata (stato parziale sulla
cartella superiore), spostamento di due dispositivi, barra Annulla.
