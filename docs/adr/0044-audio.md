# 0044 — Audio del computer controllato

**Data:** 2026-10-14 · **Stato:** accettata

## Richiesta
Sentire l'audio del computer controllato, spento di default e attivabile con un pulsante; Mac e Windows (Linux
escluso su richiesta dell'utente).

## Decisione
- **Trasporto**: un terzo data channel `audio`, aperto da chi controlla insieme a `control` e `files`, **non
  ordinato e senza ritrasmissioni** (`ordered: false`, `max_retransmits: 0`): un pacchetto in ritardo non serve,
  uno perso lo maschera il decoder. Nessuna traccia RTP e nessun cambio di SDP: le versioni precedenti ignorano il
  canale (`sort_channels` lo scarta), le nuove con un partner vecchio vedono "il partner non supporta l'audio".
- **Formato**: Opus 48 kHz stereo, pacchetti da 20 ms, 64 kbit/s (meno del 5 % del video). Sul filo: 2 byte di
  numero di sequenza (big endian) + payload Opus.
- **Cattura (host)**, crate `audio`:
  - macOS: uno stream ScreenCaptureKit solo per l'audio (video 2×2 a 1 fps ignorato), `excludesCurrentProcessAudio`
    per non rimandare indietro l'audio riprodotto dall'app; nessun permesso oltre a Registrazione schermo.
  - Windows: loopback WASAPI del dispositivo di uscita predefinito (cpal).
  - Conversione a stereo, ricampionamento lineare a 48 kHz, codifica su un thread dedicato.
- **Riproduzione (viewer)**: decodifica Opus (pacchetti persi fino a 5 mascherati con PLC, pacchetti arrivati in
  ritardo scartati), ricampionamento alla frequenza del dispositivo, coda di 60 ms prima di partire; oltre 250 ms
  in coda (deriva degli orologi, raffiche) si torna a ~60 ms.
- **Protocollo**: `Audio { enabled }` (viewer → host), `AudioUnavailable { message }` (host → viewer). Il viewer
  apre prima l'uscita audio e poi chiede l'audio all'host; spegnendo, smette subito di riprodurre.
- **Interfaccia**: pulsante "Audio" nella barra, spento a ogni sessione; non compare su Linux.
- Libopus compilata dal crate `opus` (serve `cmake` in compilazione: presente sui runner di GitHub).

## Conseguenze
- Su Linux né cattura né riproduzione (il crate `audio` lì non dipende da cpal/libopus).
- Nessun audio dalla schermata di blocco/UAC di Windows (l'app cattura nella sessione utente).
- Da verificare sul campo: qualità, ritardo e sincronia con il video su Mac e Windows.
