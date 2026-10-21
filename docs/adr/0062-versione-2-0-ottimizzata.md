# 0062 — Versione 2.0 "ottimizzata": meno CPU e RAM

**Data:** 2026-10-21 · **Stato:** accettata

## Contesto
Richiesta: rendere l'app il più leggera possibile senza rompere nulla. Misure su questo Mac (macOS, app 1.9.2/1.9.3):
- **a riposo** (app aperta, nessuna sessione): CPU ~0,3 % in tutto; memoria ~95 MB appena aperta (app 38 MB,
  pagina 33, processo grafico WebKit 17, rete 7), ~200 MB dopo ore di uso;
- **viewer in sessione**: 1.9.2 (disegno 2D) processo grafico ~15 % + app ~6 % + pagina ~1 %; 1.9.3 (WebGL) grafico
  ~10 % + pagina ~9 % + app ~7 %, con ~500 MB di memoria grafica nella pagina e ~310 MB nel processo grafico.
- Prova controllata in una finestra WebKit (2560×1440, 30 fps, codificatore compreso): WebGL con `texImage2D` ~31 %
  di CPU, disegno 2D ~47 %, WebGL con texture allocata una volta (`texStorage2D` + `texSubImage2D`) ~52 %.
- Un processo WebKit da 44 MB senza finestre, sospettato nostro, è rimasto vivo dopo la chiusura dell'app: non è
  nostro.
Analisi del codice: le copie dei fotogrammi sul computer controllato (fino a 16 MB l'una a 30 fps) sono il costo
maggiore in sessione; a riposo pesano i controlli periodici dell'interfaccia e i thread.

## Decisione
**Sessione, computer controllato**
- `capture::Frame.data` è `Arc<Vec<u8>>`: la cattura condivisa, l'ultimo fotogramma tenuto per i keyframe e la
  composizione "tutti gli schermi" (tela copy-on-write con `Arc::make_mut`) non copiano più i pixel (prima 2 copie
  per fotogramma già alla cattura, ~1 GB/s a 2560×1600).
- Windows: una sola copia dal buffer di DXGI (ridimensionamento direttamente da lì); pipe dell'agente letta con
  l'intestazione prima, i pixel direttamente nel loro buffer (prima allocazione azzerata più spostamento dell'intero
  fotogramma per togliere 9 byte).
- Riduzione alla risoluzione per poca banda (`scale::Shrinker`): pesi e buffer tenuti tra i fotogrammi.
- Statistiche CPU/RAM ogni 2 s: elenco completo dei processi ogni 30 s, in mezzo solo i nostri.
- macOS: i controlli dello schermo ogni 2 s usano CoreGraphics (`CGGetActiveDisplayList`, `CGDisplayBounds`)
  invece di `SCShareableContent`, che elenca anche tutte le finestre.

**Viewer**
- WebGL resta (`texImage2D`, la più leggera nella prova). Il bicubico di Adatta usa al massimo 2× il fotogramma e
  4K; Intero e Pixel reali usano il fotogramma alla sua dimensione, ingrandito dal compositore con
  `image-rendering: pixelated` (stessi quadrati esatti, senza un buffer grande quanto lo schermo).
- Buffer di ricostruzione dei fotogrammi già della misura del precedente.

**A riposo**
- Controlli ogni 3 s (permessi, UAC, rete locale) sospesi con la finestra nascosta e ripresi subito quando torna.
- L'orologio dell'interfaccia (prima ogni 250 ms in ogni finestra) gira solo con una richiesta di accesso aperta
  (250 ms, barra fluida) o in un viewer (1 s).
- Runtime tokio con 4 thread nell'app (prima uno per core) e 2 nell'agente Windows.
- Preferenze e dispositivi riletti dal disco solo se il file è cambiato (data di modifica).
- Avvisi mDNS di WebRTC fuori dal log (il 92 % delle righe).

**Build**: NASM installato nelle build di release (Windows, Mac Intel, Linux): OpenH264, usato quando manca il
codificatore hardware, ora compila con il suo assembly x86.

**Scartati**: chiudere la finestra principale quando è nascosta (approvazioni e banner vivono lì), memoria
condivisa con l'agente Windows al posto della pipe, `panic = "abort"`.

## Conseguenze
- Nessun cambio di protocollo: i computer 2.0 funzionano con quelli 1.9.
- Windows compilato e testato solo in CI: la conferma sul campo viene dalla VM dell'utente.
- Da misurare dopo la release con gli stessi scenari (riposo, viewer, host).
