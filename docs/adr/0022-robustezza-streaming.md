# 0022 — Robustezza dello streaming (blocchi dell'immagine)

**Data:** 2026-10-08 · **Stato:** accettata

## Problema (test utente)
L'immagine a volte si bloccava.

## Cause individuate
1. **Ritrasmissioni NACK trattate come perdite**: gli interceptor di default di webrtc-rs fanno NACK, quindi i
   pacchetti persi tornano *fuori ordine*. Il `FrameAssembler` dichiarava perdita a ogni buco di sequenza, scartava
   tutto fino a un keyframe e vedeva la ritrasmissione come un nuovo buco: spirale di keyframe (grandi, a raffica) e perdite.
2. **Attesa senza fine del keyframe**: mentre si aspettava un keyframe, i frame P venivano scartati in silenzio;
   se la richiesta capitava male non veniva ripetuta.
3. **Frame skipping di OpenH264**: con movimento e bitrate fisso l'encoder poteva saltare molti frame di fila.

## Decisione
- **Buffer di riordino** nel `FrameAssembler`: numeri di sequenza estesi (senza wrap), i pacchetti fuori ordine
  attendono; un buco diventa perdita solo dopo 200 ms o oltre 1024 pacchetti in attesa. Duplicati e pacchetti tardivi ignorati.
- In attesa di keyframe, ogni frame scartato produce `Lost` → il viewer ripete la richiesta (max 1 ogni 500 ms).
- Host: al massimo un keyframe forzato ogni 300 ms.
- Encoder: `skip_frames(false)`.
- UI: se la coda del `VideoDecoder` supera 6 frame si scarta e si riparte dal keyframe successivo (niente ritardo
  che cresce). Contatore "risinc." nella barra del viewer e "Keyframe requests" in `peer-poc` per diagnosi.

## Da fare se servisse ancora
Pacing dei pacchetti dei keyframe, controllo di congestione / bitrate adattivo (TWCC), FEC.
