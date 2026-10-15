# ADR 0050 — Online/Offline dei dispositivi salvati

**Data:** 2026-10-15
**Stato:** accettato

## Contesto
L'utente vuole vedere nella lista "I miei dispositivi" quali sono accesi. Solo il server di segnalazione sa chi è
collegato; finora lo diceva solo come errore `peer_offline` a un `connect`.

## Decisione
- Protocollo: `ClientMessage::Presence { ids }` → `ServerMessage::Presence { online }`. I server che lo supportano lo
  dicono in `Registered.features` (`"presence"`); un client nuovo con un server vecchio non chiede nulla, un client
  vecchio ignora il campo.
- Limiti contro la scansione degli ID: al massimo 20 ID per richiesta e una richiesta ogni 10 s per connessione
  (Worker e server Rust; il client aspetta 12 s per non sfiorare il limite). Con 9 cifre non serve a cercare
  dispositivi, solo a seguire una lista già nota.
- App: chiede lo stato dei dispositivi salvati subito dopo la registrazione, ogni 30 s e quando si apre la sezione
  Dispositivi; un dispositivo visto in rete locale è online anche senza server. Oltre 20 dispositivi la lista è
  chiesta a gruppi di 20, uno ogni 12 s (`next_group`): con 45 dispositivi un giro completo dura ~36 s; finché un
  dispositivo non è stato chiesto non ha etichetta (mai "Offline" per difetto). L'etichetta compare solo quando lo
  stato è noto (server raggiunto e compatibile).

## Conseguenze
- Il Worker va pubblicato prima dell'app (i client vecchi non ne risentono). Pubblicato il 2026-10-15 (versione
  85849d64) e verificato in produzione con due dispositivi di prova.
- Online vuol dire "collegato al server": un PC acceso ma senza app aperta risulta offline.
