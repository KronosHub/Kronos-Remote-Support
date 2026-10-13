# 0038 — Barra laterale nella finestra principale

**Data:** 2026-10-12 · **Stato:** accettata

## Richiesta
Barra laterale stile TeamViewer con le sezioni Dispositivi, Chat, Attività, Aggiornamenti, spostando lì i pulsanti
che erano nell'intestazione.

## Decisione
- La finestra principale diventa barra laterale (220 px, scura in entrambi i temi, voce selezionata in arancione
  Kronos) + area contenuti. L'intestazione è eliminata.
- Pagine: **Supporto remoto** (consenti il controllo + connessione, pagina iniziale), **Dispositivi** (I miei
  dispositivi + rete locale; "Seleziona" porta l'ID in Supporto remoto), **Chat** (chat delle sessioni in cui si è
  controllati; schede se più controllori), **Attività** (registro). In fondo: **Aggiornamenti** (pagina con versione,
  controllo e installazione; pallino quando c'è una nuova versione), stato del server, versione.
- La chat di chi controlla resta nella finestra della sessione. Il banner "schermo condiviso" resta in alto e il suo
  pulsante Chat apre la pagina Chat; contatore dei non letti anche sulla voce Chat.
- Quick Support: stessa barra ridotta alle sole icone (niente Dispositivi né Aggiornamenti).
- Finestre allargate: principale 1060×640 (min 880), Quick 660×640 (min 540).

## Verifica
`svelte-check` senza errori; anteprima nel browser con IPC simulato (versione completa e Quick). Da provare
nell'app vera.
