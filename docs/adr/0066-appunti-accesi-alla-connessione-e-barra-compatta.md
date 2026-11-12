# 0066 — Appunti accesi alla connessione, dati della barra nascondibili, accesso non presidiato compatto

**Data:** 2026-11-12 · **Stato:** accettata (modifica in parte ADR 0063)

## Contesto
Con la revisione di sicurezza 2.1 (ADR 0063) la condivisione degli appunti partiva spenta sul viewer e andava
accesa dalla barra a ogni sessione. L'utente la usa sempre e chiede che sia accesa di default. Chiede anche di
poter nascondere i dati della seconda riga della barra (Video, Rete, Risorse...) lasciando solo il pallino di
stato, e una riga "Accesso non presidiato" più ordinata: il testo andava a capo su cinque righe accanto ai due
pulsanti. Tra tre proposte (interruttore, riquadro di stato, compatta con menu) l'utente ha scelto la compatta.

## Decisione
- **Appunti:** `run_viewer` parte con `share_clipboard = true`. Resta il resto di ADR 0063: si condivide solo
  ciò che viene copiato dopo l'accensione (quello che era negli appunti prima, spesso una password, non parte),
  il computer controllato può rifiutare con `deny_clipboard`, e la barra li spegne.
- **Barra del viewer:** il pallino di stato è un pulsante. Un clic nasconde la riga dei dati e porta il pallino
  nella prima riga, accanto al nome del partner; un altro clic la rimette. La scelta è ricordata su questo
  computer (`localStorage` `viewStatsHidden`), come zoom e volume.
- **Accesso non presidiato:** una riga con scudo, titolo ed etichetta Attivo/Spento, descrizione sotto su tutta
  la larghezza. Da attivo, un pulsante ⋯ apre un menu con "Cambia password personale" e "Disattiva" (si chiude
  con un clic fuori o Esc); da spento, un pulsante "Attiva".

## Conseguenze
- Chi si collega condivide gli appunti senza un clic in più; chi non vuole li spegne dalla barra, oppure il
  computer controllato li vieta dalle Preferenze.
- Disattiva è un clic più lontano: meno rischio di spegnere l'accesso per sbaglio.
