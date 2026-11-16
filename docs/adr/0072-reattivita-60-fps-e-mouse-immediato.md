# 0072 — Reattività: 60 fps con codifica hardware e movimenti del mouse immediati

**Data:** 2026-11-16 · **Stato:** accettata

## Contesto
L'utente trova la reazione agli input poco pronta. Lungo il percorso: tasti e clic partono subito sul canale
`control` (separato dai file), il viewer dipinge appena decodificato (`optimizeForLatency`). I ritardi evitabili
erano due: video al massimo a **30 fps** (in media ~17 ms di attesa del fotogramma, fino a 33) e movimenti del
mouse spediti **una volta per frame di animazione** (`requestAnimationFrame`, fino a 16 ms).

## Decisione
- `StreamConfig.fps` = 60 e `keyframe_interval` = 3600 (sempre 60 s). Con un encoder **software** il limite resta
  `SOFTWARE_MAX_FPS` = 30 (`encoder_max_fps`): 60 fps raddoppierebbero la CPU del computer controllato e il
  controllo di bitrate darebbe metà bit a ogni fotogramma. La soglia `HEAVY_ENCODE` resta misurata sui 30 fps.
  `rate::fps_for` continua ad abbassare il frame rate con poca banda. Cattura X11 a 30 (polling dell'intero schermo,
  host in software).
- Viewer: `MAX_FRAMES_IN_FLIGHT` 30 (sempre ~0,5 s).
- Mouse: ogni movimento parte subito; quelli entro `MOVE_GAP_MS` (4 ms) dal precedente si fondono nell'ultimo,
  spedito allo scadere (al massimo ~250 al secondo sul canale `control`).

## Conseguenze
Con codifica hardware circa 1,5 volte la banda a parità di qualità, solo quando la rete lo permette. Su host lenti
o con encoder software (come la VM ARM in emulazione, ~31 ms a fotogramma) il video non cambia; resta il guadagno
sul mouse.
