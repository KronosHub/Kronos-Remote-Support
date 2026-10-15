# ADR 0046 — Le sessioni avviate non si chiudono quando il server perde un partner

**Data:** 2026-10-15
**Stato:** accettato

## Contesto
L'utente segnala connessioni che cadono "dopo qualche minuto". Il log del Mac controllato (2026-10-12 → 15) mostra:
- il collegamento WebSocket con il server cade da solo ogni 30–90 minuti e si riapre in ~3 s ("offline:
  connessione al server persa" → "registrato come"); Cloudflare ricicla le connessioni dei Durable Object;
- molte sessioni finiscono con `session closed by server: peer reconnected` o `peer disconnected`, mentre fino
  all'istante prima arrivano i Report del viewer ogni secondo (es. sessione 40, 14:57:54 → chiusa alle 14:58:01);
- il server chiude tutte le sessioni di un dispositivo quando il suo WebSocket cade (`peer disconnected`) o quando
  si ricollega prima che il vecchio sia chiuso (`peer reconnected`); il client obbediva e chiudeva WebRTC.

Il log del viewer (994 035 804) dice lo stesso dall'altra parte: "connessione al server persa" → "registrato"
dopo ~3 s → il server manda `peer reconnected` all'host, che chiude → il viewer vede WebRTC "disconnected" ~4 s
dopo e chiude per silenzio (es. 14:57:58 → 14:58:01 → 14:58:05 → 14:58:23). Le cadute del WebSocket avvengono
spesso nello stesso istante su entrambi i computer (10:40:22, 14:07:10): dipendono da Cloudflare, non dalla rete.

Quindi un intoppo del collegamento con il server, che non serve più dopo l'avvio, chiudeva una sessione diretta
funzionante. Anche la "sessione rimasta appesa" di ADR 0045 era in buona parte questo, letto al contrario.

## Decisione
- Il client ignora `SessionClosed` per le sessioni già avviate (presenti in `viewers` o `hosting`) se il motivo non
  è `peer left`, cioè un'uscita esplicita del partner. Durante l'autenticazione resta valido.
- Un partner sparito davvero viene comunque rilevato: `Bye`, canale chiuso, stato WebRTC o i 20 s di silenzio
  (ADR 0045).
- Sull'host, `peer left` termina con `HostCommand::PartnerLeft` ("il partner si è disconnesso") invece di
  `Stop` ("terminata da te").
- Le due sessioni parallele (2026-10-14 16:14 e 16:20) erano due "Connetti" verso lo stesso host dallo stesso
  viewer: la prima finestra non era stata chiusa. Ora "Connetti" verso un dispositivo già visualizzato porta in
  primo piano la sua finestra (`Service::viewing`, `connect_to`/`connect_device` restituiscono `false`) senza
  nuova sessione; se due connessioni si incrociano durante l'autenticazione, la più nuova chiude la vecchia
  (`ViewerCtl.partner`).
- Il server non cambia: funziona anche con i client vecchi e la correzione vale con qualunque server.

## Conseguenze
- Se il partner chiude l'app senza `Bye` e il server lo segnala, la sessione si chiude dopo ~20 s invece che subito.
- Due finestre verso lo stesso computer non sono più possibili (prima non servivano: mostravano lo stesso schermo).
