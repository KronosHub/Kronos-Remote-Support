# Novità

Le note di ogni versione compaiono nell'app, nella sezione **Aggiornamenti**, quando la versione è disponibile.

## 2.1.0 — 2026-10-29
Versione dedicata alla **sicurezza**, dopo una nuova revisione del codice.
- Collegamento più protetto: nessuno, nemmeno chi gestisce il server, può inserirsi in una sessione, e i tentativi
  di indovinare la password personale non superano più il limite. Una connessione lasciata a metà si chiude da sola.
- Chi sbaglia la password temporanea viene fermato da solo, senza che la password cambi per tutti: un estraneo con
  il tuo ID non può più impedire a chi ti assiste di collegarsi.
- **Appunti condivisi spenti all'inizio di ogni sessione**: si accendono dal pulsante **Appunti** e da quel momento
  passa solo ciò che copi dopo (non quello che avevi già copiato, per esempio una password).
- Nuove opzioni in **Supporto remoto**: chi si collega a questo computer può **condividere gli appunti** e **usare il
  Trasferimento file** (entrambe attive; toglile per vietarle).
- Trasferimento file: si ricevono solo i file chiesti, e quelli ricevuti sono segnati come scaricati da Internet
  (Windows e macOS li controllano prima di aprirli).
- **Ricollegamento automatico** con la password personale: se il computer controllato si disconnette senza che
  nessuno chiuda la sessione (logout, riavvio, rete persa) l'app si ricollega da sola per un paio di minuti.
  Su Windows dopo il logout ritrovi la schermata di accesso, e dopo l'accesso il tuo desktop con appunti e
  Trasferimento file, senza ricollegarti a mano. Serve la 2.1.0 su entrambi i computer.
- Windows: sulla schermata di accesso, prima che qualcuno entri, il Trasferimento file non è disponibile; se accede
  un utente diverso da quello del PC, chi era collegato viene scollegato.
- Windows: **Computer usato solo da remoto** vale per tutto il computer e si cambia con il permesso di
  amministratore; all'aggiornamento l'app chiede di aggiornare il servizio (pulsante **Aggiorna**).
- Altre protezioni per Windows (programmi caricati solo dalle cartelle di sistema, cartella del monitor virtuale,
  file di aggiornamento).

## 2.0.1 — 2026-10-21
- Windows installato con il pacchetto MSI: l'aggiornamento dall'app chiede il permesso di amministratore mentre
  l'app è ancora aperta, così si può approvare anche da remoto (con il controllo finestre di amministratore
  attivo); a installazione finita Kronos riparte da solo. Vale dai prossimi aggiornamenti: questo va ancora
  approvato dal PC.
- Quando il computer che stai controllando si aggiorna, la tua finestra di controllo si chiude da sola (con il
  motivo nel riepilogo) invece di restare bloccata sull'ultima immagine. Serve la 2.0.1 su entrambi i computer.

## 2.0.0 — 2026-10-21
Versione **ottimizzata**: stesse funzioni, meno lavoro per il computer.
- Il computer controllato non copia più più volte ogni immagine dello schermo: meno processore e memoria durante
  le sessioni, soprattutto su Windows.
- Visione più leggera: le modalità Intero e Pixel reali e gli schermi molto grandi usano molta meno memoria video.
- App aperta senza sessioni più tranquilla: niente controlli continui con la finestra nascosta, meno thread, file
  delle impostazioni riletti solo se cambiano, log più piccoli.
- Codifica video senza scheda grafica più veloce sui PC Windows, Mac Intel e Linux.

## 1.9.4 — 2026-10-21
- Nuova voce **Risoluzione adattata** nel menu della dimensione dell'immagine: il computer controllato porta il suo
  schermo alla risoluzione più vicina alla tua finestra (più dettaglio vero, testo più leggibile) e la ripristina a
  fine sessione. Compare solo se sul computer controllato è attivo **Computer usato solo da remoto**; servono
  entrambi i computer aggiornati.
- L'opzione **Computer usato solo da remoto (senza monitor)** ora c'è anche sul Mac.

## 1.9.3 — 2026-10-21
- **Immagine più nitida** quando il tuo schermo è più grande di quello del computer controllato: l'ingrandimento usa
  un filtro che mantiene netti i bordi del testo invece di sfocarlo.
- Nuovo menu nella barra del viewer per la **dimensione dell'immagine**: **Adatta** (riempie la finestra, come
  prima), **Intero ×N** (il multiplo esatto più grande che entra: ogni pixel diventa un quadrato perfetto, il testo
  più nitido possibile) e **Pixel reali** (1:1, con le barre di scorrimento se l'immagine non entra). La scelta
  viene ricordata. Basta aggiornare il computer da cui controlli.

## 1.9.2 — 2026-10-21
- **Trasferimento file**: se nella cartella di destinazione c'è già un elemento con lo stesso nome, l'app chiede se
  **sovrascriverlo** o **creare una copia** (per esempio "report (2).pdf"). Per sovrascrivere sul computer
  controllato serve che anche lì ci sia la 1.9.2.
- **Trasferimento file**: elenco ordinabile per nome, tipo, dimensione, data di modifica o di creazione, in ordine
  crescente o decrescente (clic sulle intestazioni delle colonne o menu "Ordina per"); l'ordine scelto viene ricordato.

## 1.9.1 — 2026-10-21
- Nella barra delle statistiche, accanto alla risoluzione, compare **GPU** se il video è compresso dal chip video
  del computer controllato o **CPU** se lo comprime il processore (servono entrambi i computer aggiornati).
- Windows usato solo con Desktop remoto (server, macchine virtuali): dopo un riavvio il computer è raggiungibile con
  Kronos anche se nessuno è mai entrato dallo schermo del computer.
- Nuova opzione **Computer usato solo da remoto (senza monitor)**, chiesta quando si attiva l'accesso non presidiato:
  chiudendo Desktop remoto senza uscire, la sessione di Windows resta visibile e controllabile con Kronos. Da non
  attivare su un PC con un monitor. Dopo l'aggiornamento premere **Aggiorna** sul controllo finestre di
  amministratore.

## 1.9.0 — 2026-10-20
- Il video del computer controllato ora è compresso dal chip video (Mac: VideoToolbox; Windows: Intel, AMD o NVIDIA
  tramite Media Foundation) invece che dal processore. Sul Mac di prova il carico è sceso da un core intero a circa
  il 7%, la fluidità è salita da 22 a 29 fotogrammi al secondo e il ritardo di circa 100 ms.
- Testo più nitido: non serve più abbassare la risoluzione per alleggerire il computer, lo schermo resta a piena
  risoluzione (fino a 2560×1600).
- Se il chip video non è disponibile l'app continua da sola con la compressione di prima.

## 1.8.7 — 2026-10-20
- Computer controllato con più monitor: staccandone uno durante la sessione, i clic del mouse potevano finire nel
  punto sbagliato (spostati in alto) sullo schermo rimasto. Ora l'app segue la nuova posizione dello schermo e il
  mouse torna preciso entro un paio di secondi.

## 1.8.6 — 2026-10-20
- Windows: aprendo **Trasferimento file** l'app si bloccava del tutto. Ora la finestra si apre normalmente.

## 1.8.5 — 2026-10-19
- Windows con Desktop remoto: se la sessione dell'utente resta aperta ma scollegata (finestra RDP chiusa senza
  uscire), il servizio non avvia più la copia per la schermata di accesso, che con lo stesso ID avrebbe disturbato
  l'app già in esecuzione. Al servizio serve un nuovo aggiornamento: Windows chiede una volta il permesso di
  amministratore (oppure premi **Aggiorna**).
- In quella situazione chi si collega legge un messaggio chiaro: la sessione di Windows è scollegata e lo schermo
  compare quando qualcuno vi si ricollega o entra dal PC. Chat e file restano disponibili.

## 1.8.4 — 2026-10-19
- Windows: con **Controllo finestre di amministratore** attivo e una password personale, il PC resta raggiungibile
  anche dopo un riavvio, quando è ancora sulla schermata di accesso. Ti colleghi con la password personale ed entri
  in Windows da remoto; la sessione continua senza interruzioni anche dopo l'accesso. Al primo avvio dopo
  l'aggiornamento Windows chiede una volta il permesso di amministratore per aggiornare il servizio (oppure premi
  **Aggiorna** accanto a Controllo finestre di amministratore).
- I miei dispositivi: in **Organizza** puoi **esportare** la lista in un file (nella cartella Download) e
  **importarla** su un altro computer. Puoi includere le password salvate: in quel caso il file è cifrato con una
  password che scegli tu.
- Sessione in rete locale rimasta ferma dopo lo stop del computer che controlla: la finestra restava aperta a
  0 kbit/s, Disconnetti non la chiudeva e non ci si poteva più collegare a quel dispositivo senza riavviare l'app.
  Ora la sessione si chiude da sola con il resoconto (o continua, se il partner c'è ancora) e ci si ricollega
  normalmente.

## 1.8.3 — 2026-10-19
- Mac controllato con il monitor in stop: collegandoti ora vedi subito la schermata con il campo della password.
  Prima si vedeva solo lo sfondo e bisognava disconnettersi e ricollegarsi: l'app scambiava il monitor spento per un
  Mac senza monitor e mostrava uno schermo virtuale. Ora risveglia il monitor e mostra quello.

## 1.8.2 — 2026-10-16
- I miei dispositivi: per mettere i dispositivi nelle cartelle ora si preme **Organizza**. Selezioni uno o più
  dispositivi (o una cartella intera), premi **Sposta in…**, scegli la cartella e confermi con **Sposta**; per qualche
  secondo puoi ancora annullare. Fuori da Organizza non c'è più il menu accanto a ogni dispositivo, così non se ne
  sposta nessuno per errore. Anche creare, modificare ed eliminare le cartelle si fa da Organizza.

## 1.8.1 — 2026-10-16
- Nuovo pulsante **Trasferimento file** nella finestra della sessione: si apre una finestra divisa in due, con le
  cartelle di questo computer a sinistra e quelle del computer controllato a destra, tutte e due sulla cartella home.
  Selezioni file e cartelle e li copi con **Invia →** o **← Ricevi** nella cartella aperta dall'altra parte, con
  barra di avanzamento e Annulla. Puoi anche creare cartelle e mostrare i file nascosti. Nessun file viene
  sovrascritto: se il nome esiste già, la copia prende un nome nuovo. Sul computer controllato l'Attività mostra
  cosa è stato copiato. Serve la versione 1.8.1 su tutti e due i computer.
- Il resoconto di fine sessione si chiude da solo dopo 30 secondi.

## 1.8.0 — 2026-10-16
- Dispositivi: in **I miei dispositivi** puoi creare cartelle, anche una dentro l'altra, e spostarci i dispositivi
  dal menu accanto a ciascuno. Le cartelle si aprono e si chiudono con un clic e mostrano quanti dispositivi sono
  online. Eliminando una cartella i dispositivi passano a quella superiore: non se ne perde nessuno.
- Kronos Quick Support: l'ID resta sempre lo stesso, anche chiudendo e riaprendo il programma.
- Kronos Quick Support: puoi impostare una password personale per entrare senza conferma (accesso non presidiato),
  finché la finestra del programma è aperta.

## 1.7.3 — 2026-10-16
- Computer controllato: se chi ti controlla si ricollega, la sessione nuova sostituisce quelle vecchie. Prima
  potevano restare aperte più condivisioni verso la stessa persona.

## 1.7.2 — 2026-10-15
- Disconnetti: il computer controllato si accorge sempre che hai chiuso la sessione, anche se nel frattempo uno dei
  due computer si era ricollegato al server (prima poteva restare in modalità controllata).
- OK sul resoconto della sessione chiude tutto ciò che ne resta: connessione e finestra di controllo.

## 1.7.1 — 2026-10-15
- Dispositivi: accanto a ogni dispositivo salvato c'è l'etichetta **Online** (verde) o **Offline** (grigia), così
  vedi subito quali sono accesi e raggiungibili. Si aggiorna da sola ogni 30 secondi e quando apri la sezione;
  con più di 20 dispositivi lo stato arriva a gruppi di 20, in pochi secondi.

## 1.7.0 — 2026-10-15
- Windows, monitor virtuale: nella sezione **Aggiornamenti** puoi installare uno schermo virtuale da 1920×1080
  sui PC senza monitor collegato, così chi si collega vede e controlla il desktop. Windows chiede il permesso di
  amministratore e una conferma per il driver (Virtual Display Driver, open source, firmato da SignPath
  Foundation). Dalla stessa sezione lo aggiorni quando ne arriva una versione nuova o lo rimuovi.
- Lo stesso è disponibile da solo come `KronosVirtualMonitor.exe` tra i file della versione: doppio clic e scegli
  Installa o Rimuovi.

## 1.6.0 — 2026-10-15
- Mac senza monitor (per esempio un Mac mini senza schermo collegato): l'app crea da sola uno schermo virtuale
  da 1920×1080 per la durata della sessione, così lo puoi vedere e controllare. Lo schermo sparisce quando la
  sessione finisce.
- Computer senza schermo da condividere (per esempio un PC Windows senza monitor): la sessione non si chiude più
  dopo 2 secondi. Resta aperta con il messaggio che spiega cosa succede, puoi usare chat e appunti, e lo schermo
  compare da solo appena ne viene collegato uno. Vanno aggiornati entrambi i computer.

## 1.5.4 — 2026-10-15
- Appunti condivisi: ora passano anche le immagini, per esempio uno screenshot fatto con ⌘⌃⇧4 sul Mac o con
  Stamp/Win+Shift+S su Windows. Si incollano direttamente come immagine dall'altra parte. Funziona tra Mac e
  Windows (su Linux le immagini non vengono condivise); vanno aggiornati entrambi i computer.

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
