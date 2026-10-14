# Novità

Le note di ogni versione compaiono nell'app, nella sezione **Aggiornamenti**, quando la versione è disponibile.

## 1.2.5 — 2026-10-14
- "Resync per minuto" ora cala davvero dopo un minuto senza resync (nella 1.2.4 restava fermo).
- Le scorciatoie prese dal tuo computer (es. ⌘⇧4 per lo screenshot, ⌘Tab) non lasciano più ⌘ o ⇧ "premuti" sul
  computer controllato: prima clic e tasti diventavano scorciatoie e sembrava tutto bloccato.
- Immagine più nitida sugli schermi grandi e Retina: con la rete buona fino a 2560×1600 (prima al massimo 1920×1080).
  Se il computer controllato fatica a stare dietro, torna da solo a 1080p.
- Quando la banda cala e la risoluzione viene ridotta, il testo resta più leggibile (riduzione più accurata).

## 1.2.4 — 2026-10-14
- "Resync per minuto" è sempre visibile, anche quando è 0, e conta solo l'ultimo minuto: cala da solo quando la
  rete torna stabile (prima restava fermo all'ultimo valore).
- Mac: i clic del mouse funzionano anche dopo essere passati dalla schermata di blocco o di accesso (prima il
  puntatore si muoveva ma i clic andavano persi).

## 1.2.3 — 2026-10-14
- Collegandosi a un computer fermo sulla schermata di accesso o di blocco, dopo aver inserito la password
  l'immagine riprende da sola, senza doversi riconnettere (Mac e Windows).
- Se lo schermo del partner cambia risoluzione o la cattura si interrompe, la trasmissione riparte da sola.
- Tenendo premuto un tasto (es. una freccia) il tasto si ripete anche sul computer controllato.
- Durante il controllo remoto il computer controllato non va più in stop né si blocca per inattività.
- Barra delle statistiche: contorno giallo quando un valore merita attenzione, rosso quando è critico; i valori
  non vengono più troncati con i puntini; "Risinc." diventa "Resync per minuto"
  (quante volte nell'ultimo minuto l'immagine è stata ricostruita per dati persi in rete).
- Puntatore del mouse normale (freccia) al posto del mirino sullo schermo del partner.

## 1.2.2 — 2026-10-14
- Con poca banda (sotto 0,8 Mbit/s) lo schermo del partner viene trasmesso a risoluzione ridotta, così il testo
  resta leggibile; torna piena quando la rete migliora.
- La finestra di controllo si riempie: allargandola, lo schermo del partner si ingrandisce in proporzione.
- Barra delle statistiche con campi di larghezza fissa: non si spostano più al cambiare dei valori.
- Aggiornamenti: l'app mostra le novità di tutte le versioni uscite dopo quella installata.

## 1.2.1 — 2026-10-13
- Il codice dell'app è pubblico (licenza MIT): github.com/KronosHub/Kronos-Remote-Support.
- Gli aggiornamenti arrivano dal nuovo indirizzo, con il vecchio come riserva.

## 1.2.0 — 2026-10-13
- Windows: il servizio per l'accesso non presidiato usa una copia protetta dell'app in Programmi.
- La password temporanea cambia da sola dopo 3 tentativi errati.
- Password personale: dopo 5 tentativi errati in 10 minuti l'accesso non presidiato si blocca fino a 10 minuti.
- Chat: limiti alla lunghezza dei messaggi e agli invii troppo ravvicinati.
- Appunti: i file ricevuti vengono scartati se la condivisione è spenta.
- Rete locale: limite al numero di connessioni contemporanee.

## 1.1.1 — 2026-10-12
- Rete locale: l'app prova tutti gli indirizzi del computer e, se non riesce, si collega tramite server.
- Corretto il messaggio "l'utente remoto ha rifiutato" dopo aver accettato una connessione in rete locale.
- macOS: richiesta del permesso "Rete locale".

## 1.1.0 — 2026-10-12
- Nuova barra laterale: Supporto remoto, Dispositivi, Chat, Attività, Aggiornamenti.

## 1.0.0 — 2026-10-09
- Chat tra chi assiste e chi viene assistito.
- Prima versione stabile.
