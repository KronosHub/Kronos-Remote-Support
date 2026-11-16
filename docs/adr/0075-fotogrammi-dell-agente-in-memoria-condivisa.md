# 0075 — Fotogrammi dell'agente in memoria condivisa

**Data:** 2026-11-16 · **Stato:** accettata

## Contesto
Con la 2.3.5 sulla VM Windows (ARM, app x64 in emulazione, schermo 5760×3240 inviato a 2560×1440) il ritardo
era minore ma ancora visibile e l'immagine andava a circa 5 fps. Misure sulla VM con esempi in
`crates/agent/examples`:
- cattura DXGI dell'agente (`capture_bench`): ~50 ms a fotogramma, ~18 fps;
- riduzione 5760×3240 → 2560×1440: ~31 ms;
- un fotogramma 2560×1440 (14,7 MB) sulla pipe tokio: 30–90 ms. mio legge le named pipe a blocchi di **4 KB**
  (un'operazione overlapped e un risveglio per blocco, ~3600 per fotogramma).

## Decisione
- `StartCapture.shared_memory` (l'app lo chiede su Windows): l'agente crea una sezione senza nome di
  `larghezza × altezza × 4` byte, la mappa in scrittura e ne duplica un handle **in sola lettura** direttamente nel
  processo dell'app (`DuplicateHandle` verso il PID del client della pipe, già verificato da `client_allowed`).
  `CaptureStarted.shared` porta l'handle; ogni fotogramma è un `Reply::SharedFrame { width, height }` sulla pipe.
- Solo con la cattura a ritmo (ADR 0073): l'agente riscrive la sezione solo dopo `FrameDone`, quando l'app l'ha già
  copiata, quindi basta un buffer.
- Compatibilità: un agente vecchio ignora il campo e manda i pixel sulla pipe; l'app accetta entrambi. Se la sezione
  non si crea o un fotogramma non ci sta, quel fotogramma passa dalla pipe.
- `SERVICE_REVISION` 8.

## Conseguenze
Sulla VM (`agent_bench`, schema di prova 2560×1440, percorso completo agente → app): **30 → 73 fps**. Niente nome
da indovinare: solo l'app verificata riceve l'handle, e senza diritto di scrittura.
