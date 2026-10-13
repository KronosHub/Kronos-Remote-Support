# 0028 — Qualità adattiva alla banda disponibile

**Data:** 2026-10-08 · **Stato:** accettata

## Problema
Richiesta utente: abbassare la qualità dello streaming quando la banda non basta, invece di perdere pacchetti
(immagine che si blocca e si risincronizza di continuo).

## Misure preliminari (encoder)
- Con `skip_frames(false)` (ADR 0022) e QP minimo 0, il rate control di OpenH264 **non rispettava il bitrate**: schermo
  "occupato" 1280×720 → 3,3 Mbit/s con obiettivo 1 Mbit/s e anche 0,3 Mbit/s. Con QP minimo ≥ 1 segue l'obiettivo
  (1,06 Mbit/s per 1 Mbit/s); sotto ~1 Mbit/s a 30 fps arriva al QP massimo e non scende oltre.
- Con lo skip di OpenH264 attivo il bitrate è rispettato, ma a 0,3 Mbit/s si arriva a 0 fps (= blocco).

## Decisione
- **Encoder**: QP minimo 10; `H264Encoder::set_rate(bitrate, fps)` cambia al volo bitrate e frame rate
  (`ENCODER_OPTION_BITRATE`, `ENCODER_OPTION_FRAME_RATE`), con tetto il bitrate iniziale (4 Mbit/s).
- **Frame rate scelto da noi** (non dallo skip dell'encoder): ≥1,5 Mbit/s → 30 fps, ≥0,8 → 20, ≥0,45 → 15, sotto → 10.
  Il loop di codifica tiene l'ultimo frame catturato e lo codifica quando scade l'intervallo: lo stato finale dello
  schermo arriva sempre.
- **Feedback**: il viewer invia ogni secondo `report {received, lost, rtt_ms}` (pacchetti video ricevuti / persi
  definitivamente dal `FrameAssembler`, RTT del ping sul data channel).
- **Regolatore lato host** (`stream::rate`, tipo AIMD):
  - congestione = perdita > 2 % (almeno 10 pacchetti e 2 persi) oppure coda > 120 ms (RTT minore degli ultimi 2
    campioni meno il minimo degli ultimi 30): bitrate ×0,8; grave (perdita > 10 % o coda > 300 ms): ×0,6;
    al massimo una riduzione ogni 1,5 s; minimo 250 kbit/s;
  - rete pulita (perdita < 1 %, coda < 50 ms, video che scorre) e 4 s dall'ultima riduzione: +12,5 % + 50 kbit/s al secondo
    fino al massimo.
  - all'avvio si parte dal massimo: su rete buona la qualità piena è subito disponibile.
- Host → viewer `quality {kbps, fps, limited}`; l'app mostra "qualità ridotta (x Mbit/s)" nella barra del viewer.
- Compatibile con versioni precedenti: messaggi sconosciuti ignorati da entrambi i lati (senza report l'host resta al massimo).

## Bug trovato durante i test
Il buffer di riordino (ADR 0022) contava i 200 ms di attesa **da quando un buco diventava il primo**, non dall'arrivo
dei pacchetti successivi: con perdita costante i buchi si mettevano in coda (uno ogni 200 ms) e l'immagine accumulava
secondi di ritardo, con perdite "fantasma" anche dopo la fine della perdita reale. Ora l'attesa parte dall'arrivo del
pacchetto più vecchio in coda. Test di regressione `steady_loss_does_not_build_latency` (fallisce sul codice vecchio).

## Verifica
- Hook di test `KRS_TEST_PACKET_LOSS=<percento>:<secondi>` (scarta pacchetti video in ricezione).
- 15 % di perdita per 12 s: 4 → 0,25 Mbit/s (10 fps) in ~10 s, ritorno a 4 Mbit/s 16 s dopo la fine; risinc. da 47 a 22.
- Relay TURN reale, rete pulita: nessuna variazione, 29 fps, 0 risinc.

## Possibili sviluppi
Riduzione della risoluzione sotto i 250 kbit/s; encoder hardware; stima della banda lato invio (TWCC).
