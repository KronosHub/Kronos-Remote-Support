# 0058 — Codifica video hardware con un FFmpeg minimo

**Data:** 2026-10-20 · **Stato:** accettata (v1.9.0)

## Problema
Il video del computer controllato era compresso solo da OpenH264, sul processore. Su questo Mac (schermo 4608×2880,
cattura a 2560×1600) un fotogramma richiedeva 28–45 ms: un core sempre occupato, 22–23 fotogrammi al secondo e,
dopo 2 s, la risoluzione ridotta a 1920×1080 (`HEAVY_ENCODE`), con il testo sfocato rispetto a VNC. Richiesta
dell'utente: usare il chip video e alleggerire il carico sul computer.

## Decisione
- **FFmpeg minimo, statico, LGPL** (`tools/ffmpeg/build.sh`, versione 7.1.2 con SHA-256 fissato): solo libavcodec e
  libavutil (più libswscale su Windows), con il solo codificatore hardware H.264 della piattaforma:
  `h264_videotoolbox` sul Mac, `h264_mf` (Media Foundation: Intel, AMD, NVIDIA) su Windows. Compilato nei workflow
  (con cache) e collegato quando c'è `FFMPEG_DIR`; senza, l'app ha solo OpenH264 come prima. Linux per ora OpenH264.
- **Due patch** (`tools/ffmpeg/patches/`), solo quando chiediamo bassa latenza (`AV_CODEC_FLAG_LOW_DELAY`):
  il pacchetto di un fotogramma esce subito (VideoToolbox: `CompleteFrames` fino a quel fotogramma; Media Foundation:
  eventi già pronti letti senza bloccare, `AVLowLatencyMode`), invece che con il fotogramma successivo; e il bitrate
  cambiato tra un fotogramma e l'altro arriva al codificatore in corsa (`AverageBitRate`/`DataRateLimits`,
  `CODECAPI_AVEncCommonMeanBitRate`/`MaxBitRate`), che il controllo adattivo usa di continuo.
- **Strato C** (`crates/codec/native/hwenc.c`): apre, cambia il bitrate, codifica un'immagine, chiude; il Rust non
  dipende dalle strutture di FFmpeg. `codec::VideoEncoder` è l'interfaccia comune; `codec::new_encoder` prova
  l'hardware e ricade su OpenH264 (anche per il resto dello stream se l'hardware rifiuta).
- **Mac senza copie**: con l'hardware la cattura consegna il `CVPixelBuffer` di ScreenCaptureKit
  (`CaptureConfig.native`, `Frame.native`) e il codificatore lo legge così com'è, conversione dei colori compresa;
  la copia in BGRA (`Frame::pixels`) avviene solo se serve (immagine ridotta per poca banda, OpenH264). Coda di
  ScreenCaptureKit portata a 6 buffer, perché i fotogrammi tenuti (in attesa, ultimo per i keyframe) non la esauriscano.
- **Windows**: BGRA → NV12 con swscale (SIMD), Media Foundation in modalità "display remoting", rate control a picco
  limitato; attesa del pacchetto fino a 60 ms con timer da 1 ms.
- Con l'hardware il limite di risoluzione per "codifica lenta" non scatta più.

## Misure (questo Mac, 2560×1600, finestra animata, 20 s)
| | OpenH264 | VideoToolbox |
|---|---|---|
| Processore del processo | 20,3 s (un core) | 1,2–1,6 s (~7% di un core) |
| Fotogrammi al secondo | 22–23 | 29,4 |
| Latenza cattura → pacchetto | 113–117 ms | ~17 ms |
Servizio di sistema `VTEncoderXPCService`: 0 s (su Apple Silicon la codifica è nel processo). Il flusso del chip
(profilo High, `avc1.640032`) si decodifica con WebCodecs senza errori (Chrome, 2560×1600).

## Conseguenze
- L'app cresce di qualche MB; licenza: FFmpeg LGPL collegato staticamente, sorgente e patch nel repository (MIT),
  ricompilabile da chiunque.
- La prima prova del codificatore Windows è sul PC dell'utente (i runner di GitHub non hanno GPU: lì il test si salta).
- Restano possibili: conversione NV12 sulla GPU su Windows, cattura DXGI senza copia, VAAPI su Linux.
