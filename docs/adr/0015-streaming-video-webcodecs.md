# 0015 — Streaming video: traccia WebRTC H.264 + decodifica WebCodecs nella UI

**Data:** 2026-10-07 · **Stato:** accettata (modifica l'ADR 0005)

## Decisione
- **Host**: cattura (`capture`) → H.264 OpenH264 (`codec`) su thread dedicato → `TrackLocalStaticSample`
  WebRTC (crate `stream`). Profilo Constrained Baseline, `packetization-mode=1`, 1920x1080 max, 30 fps, 4 Mbit/s,
  keyframe periodico ogni 300 frame + su richiesta.
- La traccia video dell'host viene aggiunta **prima** della risposta SDP; il viewer (offerer) dichiara un
  transceiver video `recvonly`.
- **Viewer (Rust)**: `FrameAssembler` ricompone gli access unit dai pacchetti RTP (depacketizer H.264 di `rtc`),
  rileva perdite (salto di sequenza o timestamp cambiato a frame incompleto) e scarta fino al keyframe successivo.
- **Richieste di keyframe** sul canale di controllo (`ControlMsg::RequestKeyframe`, crate `protocol`), non via RTCP PLI:
  webrtc-rs 0.21 non consegna l'RTCP all'applicazione con la catena di interceptor di default.
- **Decodifica nella UI con WebCodecs** (`VideoDecoder`, accelerazione hardware), frame passati dal backend via
  `tauri::ipc::Channel` binario: `[keyframe u8][timestamp µs u64 LE][Annex B]`. Il codec string (`avc1.PPCCLL`)
  si ricava dall'SPS del primo keyframe.

## Perché non decodificare in Rust (ADR 0005)
Passare frame RGBA alla webview costa ~240 MB/s a 1080p30; l'H.264 compresso pochi Mbit/s.
WebCodecs è disponibile in WebView2 (Windows) e WKWebView (macOS).

## Rischi
- **Linux (WebKitGTK)**: supporto WebCodecs da verificare; se manca, fallback futuro: decodifica OpenH264 in Rust
  e frame ridotti/JPEG, oppure finestra nativa.
- Una sola traccia/encoder per sessione: più viewer contemporanei = più encoder.
