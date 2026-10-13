# 0034 — Collegamento in rete locale senza server

**Data:** 2026-10-09 · **Stato:** accettata

## Richiesta
Sulla stessa rete i PC si devono collegare senza passare dal server.

## Decisione
- **Scoperta mDNS** (`mdns-sd`, servizio `_kronos-rs._tcp`): ogni app annuncia il proprio ID (TXT `id`, `name`) e la
  porta di un listener TCP locale; sfoglia per trovare gli altri (esclude sé stessa). `transport::lan::LanDiscovery`.
- **Signaling diretto**: il viewer apre una connessione TCP verso l'host trovato; i due si scambiano offerta, risposta
  e candidati ICE come JSON per riga (`Hello`, `Signal`, `Bye`). `SignalSender::channel()` permette di pilotare un
  `PeerSession` senza server; nessun server ICE (in LAN basta il candidato host).
- Dopo la connessione tutto è identico al percorso col server: handshake con password (SPAKE2) e conferma sull'host,
  streaming, appunti. Il `Hello` dichiara l'ID solo a scopo di visualizzazione; l'autenticazione resta la password.
- **App**: listener e scoperta partono all'avvio, indipendenti dal server. L'ID del dispositivo (assegnato dal server,
  non derivabile) viene memorizzato in `prefs.json` dopo la prima registrazione, così la LAN funziona anche col server
  irraggiungibile. Alla connessione: se l'ID è visibile in LAN → collegamento diretto, altrimenti server. UI: sezione
  "In rete locale" con i dispositivi trovati. Quick Support non avvia connessioni in uscita (blocco anche offline).

## Verifica
- Scoperta reale tra due istanze sulla stessa macchina (test `discovers_a_peer`, ignorato in CI: serve multicast).
- `tests/lan_webrtc.rs`: due `PeerSession` reali completano l'handshake WebRTC solo tramite il link LAN e aprono il
  canale dati (1,6 s). Il test ha scoperto un bug: i byte arrivati insieme all'`Hello` venivano persi.

## Limiti
- Al primo uso i firewall chiedono di consentire le connessioni in ingresso (Windows Defender, macOS).
- Il dispositivo deve essersi registrato almeno una volta col server per conoscere il proprio ID.
- Reti che bloccano il multicast (alcune Wi-Fi ospiti/aziendali) impediscono la scoperta: si usa il server.
