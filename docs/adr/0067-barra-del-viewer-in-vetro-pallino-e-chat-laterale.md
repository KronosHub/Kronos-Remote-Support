# 0067 — Barra del viewer in vetro, pannello del pallino e chat laterale

**Data:** 2026-11-12 · **Stato:** accettata

## Contesto
La barra del viewer occupava due righe sopra lo schermo del partner. L'utente vuole più spazio per lo schermo,
un aspetto più gradevole e "liquido" e una chat che non sia un riquadro fisso. Le proposte sono state provate su
due pagine di esempio ("Barra del viewer nascosta", "Chat del viewer"), con molti ritocchi: niente bordi doppi
sul pallino, pallino centrato, chiusura del pannello in un unico movimento, "Ripristina barra" che parte dal
pannello aperto, stesso vetro per barra e pannello, tema chiaro e scuro.

## Decisione
- **Barra:** vetro (sfondo traslucido, `backdrop-filter`, riflesso fermo) che galleggia sopra lo schermo, a 8 px
  dai bordi. Prima riga: pallino, nome, ID, **puntina** (blocca la barra) e **freccia** (nasconde); azioni in un
  gruppo con icone (Controllo, Blocca input, Chat, Appunti, File, Audio, HD), Adatta, Disconnetti rosso tenue.
  Seconda riga: i numeri separati da linee sottili. Terza riga solo durante un trasferimento di file.
- **Nascosta da sola dopo 30 s** dal primo fotogramma, se non è bloccata (non mentre il puntatore è sopra); le
  prime 3 volte un fumetto dice di passare sul pallino. Con la puntina la barra resta e l'immagine parte sotto di
  lei (uno spazio vuoto della sua altezza), così non copre il partner. Puntina ricordata (`viewBarPinned`).
- **Pallino:** un cerchio di 18 px in alto a sinistra. È la stessa forma del pannello: chiuso, un `clip-path`
  mostra solo il cerchio; passandoci sopra si allarga nel pannello con nome, tipo di collegamento, un giudizio in
  parole, sei valori (latenza, fotogrammi, video con GPU/CPU, ricezione, resync al minuto, CPU tu/partner) e
  quattro azioni (Chat, Trasferimento file, Ripristina barra, Disconnetti). Al posto dei "pacchetti persi" delle
  prove ci sono i resync al minuto, che l'app misura già.
- **Ripristina barra:** la barra parte con la dimensione del pannello aperto e si allarga, il pannello sfuma.
- **Chat:** cassetto di vetro ancorato al bordo destro (290 px), che scorre fuori lasciando una linguetta. Il primo
  messaggio del partner la apre senza prendere la tastiera; se l'utente l'ha nascosta, i messaggi accendono la
  linguetta (arancione che pulsa, numero) e mostrano per 4,5 s l'inizio del messaggio. Con la chat aperta la
  barra si ferma prima e mostra solo le icone. Invio spedisce, Maiusc+Invio va a capo.
- **Linguetta spostabile:** si trascina in alto o in basso (o frecce su/giù con la tastiera); l'altezza è
  ricordata (`chatTabY`) e il fumetto dei messaggi la segue. Un trascinamento non apre la chat.
- **Chat fissata:** una puntina nell'intestazione la tiene aperta (anche nelle sessioni successive, `chatPinned`)
  e lo schermo del partner si stringe nello spazio a sinistra: la tela e quindi il mouse seguono la nuova area,
  e "Risoluzione adattata" chiede la nuova dimensione.
- **Animazioni:** molle come `linear()` calcolate all'avvio (con ripiego `cubic-bezier`); si animano solo
  `transform`, `opacity` e il `clip-path` del pannello (senza sfocatura di sfondo finché è chiuso: Chrome non la
  ritaglia). `prefers-reduced-motion` le spegne.
- **Temi:** colori del vetro come variabili (`--g-*`, `--d-*`) nei blocchi chiaro e scuro dell'app.

## Conseguenze
- Senza puntina la barra copre la fascia alta dello schermo nei primi 30 s e quando viene ripristinata.
- Il pulsante "pallino che nasconde i dati" di ADR 0066 non esce mai in una release.
