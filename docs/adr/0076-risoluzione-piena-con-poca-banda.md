# 0076 — Risoluzione piena con poca banda, tetto dopo la congestione

**Data:** 2026-11-17 · **Stato:** accettata (sostituisce la riduzione di risoluzione per banda dell'ADR 0041)

## Contesto
Collegati al Mac mini via internet il testo era illeggibile: 946×532 allargato a 2576 px. Log dell'host:
- la qualità adattiva saliva da 250 kbit/s a 5 Mbit/s in ~15 s, la linea in uscita del Mac mini perdeva pacchetti,
  il bitrate ricrollava, e così ogni ~30 s;
- ogni gradino di banda cambiava risoluzione (63 volte in 30 minuti), e ogni cambio apre un nuovo encoder che parte
  con un keyframe: raffiche che riempivano la coda (RTT oltre 1 s) e causavano altre perdite e resync (29–40/min).

Misure in locale (`crates/codec/examples/refine.rs`, screenshot reale 1920×1080 dopo uno scorrimento): VideoToolbox
~38–39 dB a 1 Mbit/s, ~50 dB a 3 Mbit/s; il problema di leggibilità era la risoluzione, non la compressione.

## Decisione
- La banda non riduce più la risoluzione: solo la lentezza dell'encoder (`HEAVY_ENCODE`) e la scelta HD del viewer.
  Con poca banda calano i fotogrammi (`fps_for`: 12 fps da 450 kbit/s, 8 da 300, 5 sotto).
- **Tetto dopo la congestione**: alla prima riduzione di un episodio il controllo ricorda il bitrate che ha
  incontrato il muro e per 60 s risale solo all'85 % di quello (`CEILING_SHARE`, `CEILING_HOLD`).
- Keyframe: al massimo uno al secondo dall'host (`MIN_KEYFRAME_GAP`) e una richiesta al secondo dal viewer.
- **Ritocco a schermo fermo**: senza nuove catture, l'ultimo fotogramma viene ricodificato fino a 6 volte ogni 200 ms
  (`REFINE_PASSES`): circa +1 dB, costo trascurabile.

## Conseguenze
Testo leggibile anche con poca banda; in movimento l'immagine è più a blocchi e meno fluida, a schermo fermo torna
nitida. Meno keyframe e niente altalena di risoluzione su linee domestiche lente.

## Il "↓" a 6–7 Mbit/s: richieste di ritrasmissione senza limite
La barra segnava 6–7 Mbit/s in arrivo dal Mac mini mentre il video era a ~0,3–0,4 Mbit/s (pacchetti nei report).
Il contatore è giusto, per connessione (`get_stats().transport()`, socket propri per ogni connessione): erano
doppioni. Il generatore NACK predefinito di webrtc-rs chiede ogni pacchetto mancante **ogni 100 ms senza limite**
(`max_nacks_per_packet` 0) finché resta nel registro di 512 pacchetti, e il responder dell'host lo rispedisce ogni
volta. Con 1 s di andata e ritorno ogni perdita torna circa dieci volte; i doppioni non contano come video ricevuto
(`FrameAssembler` li scarta) ma riempiono la linea e causano altre perdite.

Simulazione `crates/transport/tests/nack_storm.rs` (30 pacchetti/s da 1200 byte, 500 ms per verso, 10 % di
perdite): predefinito 264 pacchetti rispediti per 24 persi (11 a perdita, più dei 243 originali); con il nostro
generatore (`transport::nack_generator`: al massimo 2 richieste per pacchetto, 100 ms di distanza) 48. Oltre i 200 ms
l'immagine rinuncia comunque al buco (`MAX_REORDER_WAIT`) e chiede un keyframe.

I due interceptor NACK sono configurati da noi (`session::interceptors`), il resto come in
`register_default_interceptors`.
