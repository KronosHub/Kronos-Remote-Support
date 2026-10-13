# 0020 — Più monitor: scelta dello schermo condiviso

**Data:** 2026-10-07 · **Stato:** accettata

## Contesto
Test utente: da Windows si vedeva solo uno dei due monitor del Mac (si catturava solo il principale).

## Decisione
- `capture::displays()` elenca i monitor con id, nome, principale e `bounds` nelle coordinate dell'input
  (macOS: punti globali da `SCDisplay::frame`; Windows: pixel del desktop virtuale da `GetMonitorInfoW`;
  X11: radice = tutti i monitor insieme). Ordine: principale per primo, poi da sinistra a destra.
- Protocollo: host → viewer `Displays { displays, current }`; viewer → host `SelectDisplay { id }`.
- Cambio monitor: l'host ferma l'encoder corrente, ne avvia uno sul nuovo monitor (stessa traccia video,
  nuovo keyframe e nuova risoluzione), aggiorna l'area dell'input; se fallisce ripristina il precedente.
- Input: le coordinate normalizzate si mappano sul rettangolo del monitor condiviso.
  Windows usa `SetCursorPos` (desktop virtuale) perché i movimenti assoluti di enigo coprono solo il principale.
- UI: menu "Schermo" nella barra del viewer (solo se più di un monitor); il decoder WebCodecs riparte quando
  cambia la risoluzione.

## Verifica
Mac con due monitor (esterno 1920x1080 principale + Retina 3456x2234): cambio al volo, 114 + 99 frame tutti
decodificabili, 2 keyframe. Windows/Linux: compilazione e smoke CI verdi (un monitor sui runner).
