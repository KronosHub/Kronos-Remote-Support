# 0019 — Cattura schermo su Windows e Linux

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
- **Windows**: DXGI Desktop Duplication (crate `windows-capture` 2, API sincrona `acquire_next_frame`)
  del monitor principale, BGRA. `AccessLost` (UAC, cambio risoluzione) ⇒ ricreazione; frame con
  `LastPresentTime == 0` (solo puntatore) ignorati. Preferita a Windows.Graphics.Capture: nessun bordo giallo,
  modello "pull" uguale agli altri backend, approccio usato da RustDesk.
- **Linux X11**: `GetImage` (x11rb, puro Rust) della finestra radice, ritmo fps fisso, scarto dei frame identici.
  Wayland rilevato (`XDG_SESSION_TYPE`) e segnalato come non supportato.
- **Ridimensionamento**: bilineare su CPU (`capture::scale`) verso il riquadro 1920x1080, perché questi backend
  catturano a risoluzione nativa (su macOS scala ScreenCaptureKit).
- **Verifica**: workflow `platform-smoke.yml` su GitHub: Xvfb + xdotool (Linux) e runner Windows eseguono
  `capture-poc` e l'esempio `input/smoke` (puntatore spostato e verificato).

## Limiti / evoluzioni
- Il puntatore non è nei frame Windows/X11 (il viewer vede il proprio cursore sul canvas).
- X11 senza MIT-SHM: ~8 MB per frame sul socket; ottimizzabile con SHM o XDamage.
- Wayland: xdg-desktop-portal ScreenCast + PipeWire (`ashpd`), con consenso dell'utente.
