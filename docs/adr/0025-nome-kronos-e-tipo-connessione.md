# 0025 — Nome "Kronos Remote Support", icona Kronos, tipo di connessione visibile

**Data:** 2026-10-08 · **Stato:** accettata (richieste utente)

## Decisione
- Nome visibile: **Kronos Remote Support** (finestra, menu, messaggi, installer, README). Icona ridisegnata
  sul marchio Kronos: tassello bianco con bordo arancio `#f39200`, K (asta scura `#1f1f1f`, chevron arancio) e, accanto, un monitor con due frecce opposte per indicare il controllo remoto (v0.7.2, richiesta utente).
  Accento dell'interfaccia in arancio Kronos.
- Invariati: identificativo `tech.kronos.desktopremoto` (cartella dati, ID dispositivo, dispositivi salvati,
  permessi), nomi dei crate e del binario.
- **Tipo di connessione** mostrato nella barra del viewer e nel banner dell'host: "Diretta (rete locale)",
  "Diretta (Internet)", "Relay TURN". Calcolato dalla coppia di candidati ICE selezionata
  (`SessionInfo::connection_path`, aggiornato ogni 3 s).

## Conseguenze
- Su Windows il nome dell'installer cambia: la versione "Desktop Remoto" precedente va disinstallata a mano.

## Aggiornamento — indirizzo del server non visibile (richiesta utente)
L'app può essere distribuita a terzi: nessun indirizzo del server deve comparire nell'interfaccia.
- `privacy::redact` toglie URL (`ws/wss/http/https/turn/stun`), nomi `*.sslip.io` con l'IP e indirizzi IPv4 da ogni
  messaggio diretto alla UI (registro Attività, stato). Errori di connessione mostrati come frasi brevi
  ("il server non risponde", "nessuna connessione a Internet"...); i dettagli restano nel log interno.
- Limite: senza un dominio, il nome `<ip-con-trattini>.sslip.io` della VM è comunque contenuto nel binario (superato
  dalla v0.12.0: il signaling passa dal Worker Cloudflare, ADR 0035). Con un dominio
  (meglio se dietro proxy Cloudflare per il signaling) l'IP non sarebbe più deducibile dal nome. Il relay TURN usa
  UDP e non può stare dietro proxy: il suo indirizzo è visibile a livello di rete a chi usa il relay.
