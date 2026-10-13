# 0009 — Cattura e encoding (Fase 1)

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
- **macOS**: ScreenCaptureKit tramite crate `screencapturekit` (v11, feature `macos_14_0`).
  Requisito minimo: macOS 14.
- **Formato comune** tra backend: BGRA compatto (stride = larghezza × 4), dimensioni pari.
- **Encoding**: OpenH264 (crate `openh264`, compilato da sorgente), `UsageType::ScreenContentRealTime`,
  rate control a bitrate, frame skip abilitato.
- Canale cattura→encoder con profondità 2 e scarto dei frame vecchi: per il controllo remoto conta
  il frame più recente, non lo storico.
- Si codificano solo i frame con contenuto cambiato (ScreenCaptureKit segnala i frame `Idle`).

## Conseguenze
- Il bridge Swift di `screencapturekit` richiede rpath `/usr/lib/swift`: impostato in `.cargo/config.toml`.
- Conversione BGRA→I420 su CPU: ottimizzabile in futuro chiedendo NV12 direttamente a ScreenCaptureKit
  o passando a encoder hardware (VideoToolbox).
- OpenH264 accetta al massimo 3840x2160: la cattura viene ridotta per stare in un riquadro
  (`CaptureConfig::max_size`), default 1920x1080.
- Misure (DIARIO 2026-10-07): 1080p ~17 ms/frame ok per 30 fps; 3340x2160 ~42 ms/frame non regge 30 fps.
