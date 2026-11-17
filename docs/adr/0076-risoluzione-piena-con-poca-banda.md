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

## Da capire
Durante la sessione la barra del viewer segnava 6–7 Mbit/s in arrivo dal Mac mini mentre il video era a ~0,3–0,4
Mbit/s (pacchetti nei report). Sullo stesso Mac era aperta un'altra sessione in rete locale: verificare se il
contatore di `get_stats().transport()` include altro traffico.
