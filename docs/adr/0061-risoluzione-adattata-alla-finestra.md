# 0061 — Risoluzione del computer controllato adattata alla finestra

**Data:** 2026-10-21 · **Stato:** accettata

## Contesto
Con lo schermo del viewer più grande di quello del computer controllato l'ingrandimento, anche nitido (ADR 0060),
non aggiunge dettaglio. Solo cambiando la risoluzione del computer controllato si hanno più pixel veri. Su un
computer con qualcuno davanti al monitor non si può fare. L'utente l'aveva prima esclusa, poi accettata a due
condizioni: solo con l'opzione **"Computer usato solo da remoto"** (ADR 0059) e come scelta facoltativa accanto
alle altre modalità.

## Decisione
- **Opzione**: `Prefs.remote_only` ora esiste anche sul Mac (casella in Accesso non presidiato e domanda quando
  si imposta la password personale, testi diversi per Windows e Mac). Sul Mac non sposta sessioni, abilita solo
  questa funzione.
- **Protocollo** (campi nuovi, i peer vecchi li ignorano):
  - host → viewer `ResolutionAdjustable`: mandato all'inizio della sessione se l'host ha l'opzione
    (`StreamConfig.resize_display`) e il sistema lo supporta (Windows, macOS);
  - viewer → host `MatchResolution { width, height, scale }`: pixel del dispositivo dell'area del viewer e
    pixel per punto; 0×0 = torna alla risoluzione dell'utente.
- **Viewer**: quarta voce del menu della dimensione, **Risoluzione adattata**, presente solo se l'host ha mandato
  `ResolutionAdjustable` (altrimenti la scelta salvata vale come Adatta). La richiesta parte 0,8 s dopo l'ultimo
  ridimensionamento della finestra (ogni cambio costa un cambio di modalità e un keyframe) e di nuovo con HD o un
  altro monitor; lasciando la modalità si manda 0×0.
- **Scelta della modalità** (`capture::resolution::choose`): la più grande che entra nella richiesta, limitata al
  massimo dello stream (2560×1600, o 1920×1080 con HD spento), con gli stessi pixel per punto del viewer (sul Mac
  le modalità HiDPI per un viewer Retina: testo grande come sul suo schermo) e la frequenza più vicina a quella
  attuale; se nessuna entra, la più piccola.
- **Windows**: `EnumDisplaySettingsExW` sul dispositivo GDI del monitor condiviso, `ChangeDisplaySettingsExW` con
  `CDS_FULLSCREEN` (temporaneo: il registro tiene la risoluzione dell'utente); ripristino con
  `ChangeDisplaySettingsExW(nome, NULL)`.
- **macOS**: `CGDisplayCopyAllDisplayModes` (con le modalità HiDPI), `CGConfigureDisplayWithDisplayMode` con
  `kCGConfigureForAppOnly` (annullato anche se l'app si chiude o va in crash); ripristino con
  `CGRestorePermanentDisplayConfiguration`. Lo schermo virtuale del Mac senza monitor (ADR 0048) ora offre anche
  1280×720…2560×1600 oltre a 1920×1080 (iniziale).
- **Host** (`stream::resize`): dopo un cambio la cattura riparte dopo 0,8 s alla nuova dimensione (`StreamStarted`
  come per un cambio di monitor). Un contatore per monitor tra le sessioni: la risoluzione torna quella
  dell'utente quando finisce l'ultima sessione che l'ha cambiata, o quando si passa a un altro monitor.

## Conseguenze
- Provato su questo Mac con uno schermo virtuale: 2560×1440, 1440×900, 2560×1600, 800×600 e ripristino 1920×1080.
  Windows compilato ma non provato: dipende dalle modalità del driver video (VM: di solito molte).
- Il monitor virtuale di Windows (ADR 0049) ha ancora una sola modalità 1920×1080: aggiungerne altre richiede di
  verificare quale sceglie Windows all'avvio, da fare quando si potrà provare su un PC.
