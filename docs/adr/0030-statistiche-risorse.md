# 0030 — CPU, RAM e rete nella barra del viewer

**Data:** 2026-10-08 · **Stato:** accettata

## Richiesta
Mostrare accanto a fps e info di trasmissione l'uso di CPU, RAM e rete della sessione corrente.

## Decisione
- **Rete**: byte inviati/ricevuti dalle statistiche di trasporto WebRTC (`SessionInfo::traffic`, tutto il traffico
  della PeerConnection: video, controllo, input), differenza ogni secondo → "↓ x ↑ y".
- **CPU/RAM**: crate `sysinfo` (`stream::usage::UsageSampler`): processo dell'app più i processi figli (la web view è
  figlia su Windows/Linux; su macOS i processi WebKit appartengono a launchd e non sono contati). CPU come quota
  dell'intera macchina (somma / numero di core), RAM residente.
- **Partner**: l'host invia ogni 2 s `usage {cpu, ram_mb}` sul canale di controllo (cattura e codifica avvengono lì,
  è il dato più utile). Il viewer mostra "Tu: … | Partner: …".
- Sono valori dell'app, non solo della singola sessione: con una sola sessione attiva coincidono di fatto.
- Compatibilità: host vecchi non inviano `usage` → compare solo "Tu".
