# Novità

Le note di ogni versione compaiono nell'app, nella sezione **Aggiornamenti**, quando la versione è disponibile.

## 1.5.3 — 2026-10-15
- Le connessioni non cadono più dopo qualche minuto: quando uno dei due computer perdeva per un attimo il
  collegamento con il server (succede da solo ogni tanto), la sessione veniva chiusa anche se la connessione
  diretta funzionava. Ora continua; se il partner sparisce davvero, la sessione si chiude dopo 20 secondi.
- Collegandosi a un computer già aperto, l'app porta in primo piano la sua finestra invece di aprire una seconda
  sessione.
- Sul computer controllato, quando chi controlla si disconnette, il messaggio dice "il partner si è disconnesso"
  invece di "terminata da te".

## 1.5.2 — 2026-10-14
- Pulsante ✕ accanto alla barra di avanzamento per interrompere il trasferimento di un file: lo può fare chi
  invia o chi riceve, l'altro computer viene avvisato e i file arrivati a metà vengono cancellati.

## 1.5.1 — 2026-10-14
- Audio: accanto al pulsante "Audio", quando è acceso, c'è uno slider per regolare il volume (ricordato per le
  sessioni successive). Al suo posto non c'è più il pulsante "Schermo intero": si usa quello della finestra (il
  pulsante verde su Mac, "Ingrandisci" su Windows).
- Il tuo nome si cambia con "Modifica" e si conferma con "Salva" (o Invio; Esc annulla).
- Immagine più definita in movimento sugli schermi grandi: fino a 6 Mbit/s oltre il 1080p (prima 4), solo quando lo
  schermo cambia e la rete lo permette; con lo schermo fermo il consumo non cambia.
- Meno immagini complete di "rinfresco" (una al minuto invece di ogni 10 secondi): la banda risparmiata va alla
  qualità; dopo una perdita di dati l'immagine viene comunque ricostruita subito.

## 1.5.0 — 2026-10-14
- Copia/incolla di file: barra di avanzamento in alto con percentuale, dati trasferiti e velocità (nella finestra di
  controllo e, sul computer controllato, nella barra della condivisione). Limite: 500 MB e 10.000 file per copia.
- Chat più visibile: il pulsante "Chat" lampeggia quando arriva un messaggio; sul computer controllato si apre una
  piccola finestra di chat in alto a destra, sempre in primo piano, per leggere e rispondere senza cercare l'app.

## 1.4.0 — 2026-10-14
- Il tuo nome: impostalo in "Consenti il controllo". Chi ti controlla lo vede nella richiesta di accesso e durante
  la sessione; chi controlli lo vede in alto, prima del tuo ID (va aggiornato anche il partner).
- Blocca input partner: chi controlla può bloccare mouse e tastiera del computer controllato (Mac e Windows), così
  chi è davanti non interferisce. Chi è davanti al computer lo vede segnalato e può riprenderli premendo Esc tre
  volte; il blocco finisce comunque con la sessione.
- Pallino di stato accanto al tipo di connessione: verde se va tutto bene, giallo se c'è almeno un avviso, rosso se
  c'è un valore critico (passandoci sopra si vede il motivo).
- Barra della finestra di controllo su due righe: nome, ID e pulsanti sopra, connessione e statistiche sotto (più
  leggibile sugli schermi piccoli).
- "Resync per minuto" compare solo quando nell'ultimo minuto c'è stato almeno un resync.
- Se uno dei due computer non risponde più per 20 secondi la sessione si chiude da sola ("il partner non
  risponde"): prima il computer controllato poteva restare in condivisione anche dopo che chi controllava aveva
  chiuso.

## 1.3.0 — 2026-10-14
- Audio: pulsante "Audio" nella barra per ascoltare il suono del computer controllato (Mac e Windows). È spento
  all'inizio di ogni sessione e usa poca banda (circa 64 kbit/s).
- Pulsante "HD": acceso, l'immagine arriva fino a 2560×1600 con la rete buona; spento, al massimo 1080p (meno banda
  e meno lavoro per il computer controllato).
- Aggiornamenti: aprendo la sezione "Aggiornamenti" l'app controlla subito, e da sola ogni 4 ore.

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
