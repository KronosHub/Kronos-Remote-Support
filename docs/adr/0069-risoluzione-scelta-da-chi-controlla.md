# 0069 — Risoluzione del computer controllato scelta da chi controlla

**Data:** 2026-11-13 · **Stato:** accettata

## Contesto
Collegandosi a computer con schermi ad alta risoluzione (4K, 5K) da un portatile, tutto appare minuscolo. La
"Risoluzione adattata" (ADR 0061) cambia la risoluzione solo sui computer "usati solo da remoto" e la sceglie
dalla finestra. L'utente vuole scegliere lui una risoluzione qualunque, anche perdendo l'HiDPI, su qualunque
computer, con il ripristino prima della disconnessione. Pagina di prova pubblicata con tre opzioni per chi è
davanti al computer controllato; scelta la 1: **sempre permesso, con avviso e "Ripristina"**.

## Decisione
- **Protocollo** (i peer vecchi ignorano i messaggi nuovi):
  - host → viewer `DisplayModes { display, modes, current, user }` (`ScreenMode { width, height, scale }`, pixel e
    pixel per punto): all'inizio, dopo ogni riavvio della cattura o cambio di monitor e in risposta a ogni
    `SetResolution`. `modes` vuoto = non si può cambiare (niente controllo remoto, Linux, "Tutti gli schermi",
    schermo virtuale a dimensione fissa delle Preferenze);
  - viewer → host `SetResolution { width, height, scale }`; 0×0 = la risoluzione dell'utente.
- **Elenco** (`capture::resolution::modes`/`offered`): una voce per dimensione in punti, non sotto 1024×600,
  dalla più grande. Di una dimensione che esiste normale e HiDPI resta la normale (sullo schermo di chi controlla
  sono identiche, l'HiDPI costa quattro volte i pixel); l'HiDPI resta solo se è l'unica. Solo le dimensioni comuni
  (`COMMON_POINTS`: 3840×2160, 2560×1440, 1920×1080, 1600×900, 1280×720, …) se lo schermo ne ha almeno due,
  altrimenti al massimo 6 distribuite dalla più grande alla più piccola. Il menu non distingue più le HiDPI. La
  scelta passa per `choose`, che con una modalità dell'elenco la trova esatta; nessun limite allo stream (i pixel
  in più si riducono prima della codifica).
- **Permesso**: basta il controllo remoto consentito; non serve "solo da remoto". Chi è al computer controllato
  vede nel banner "Risoluzione cambiata in …" con **Ripristina** (`HostCommand::RestoreResolution`), e una riga
  in Attività.
- **Risoluzione dell'utente**: `stream::resize` la legge prima del primo cambio di ogni schermo e la tiene col
  contatore delle sessioni (ADR 0061); torna quando finisce l'ultima sessione che l'ha cambiata.
- **Disconnessione**: se lo schermo del partner non è alla risoluzione dell'utente, `run_viewer` manda
  `SetResolution 0×0`, aspetta il `DisplayModes` di risposta (al massimo 3 s, `RESTORE_WAIT`) e solo dopo manda
  `Bye`. Vale per Disconnetti e per la chiusura della finestra (entrambi fermano la sessione); la finestra mostra
  "Ripristino…" finché la sessione non è chiusa. Se la rete cade o il viewer si chiude di colpo, l'host
  ripristina comunque a fine sessione.
- **Crash**: finché uno schermo è cambiato esiste `resolution-changed` nella cartella dati; all'avvio, se c'è,
  `resolution::restore_all` rimette tutti gli schermi come nel registro (Windows) o nelle impostazioni (macOS,
  che comunque annulla da solo i cambi `kCGConfigureForAppOnly` quando l'app esce).
- **Viewer**: menu nella barra accanto a quello della dimensione, con "Risoluzione del partner (…)", le modalità
  normali e in un gruppo le HiDPI. Scegliendone una si lascia la Risoluzione adattata, che la annullerebbe al
  primo ridimensionamento della finestra. Le etichette sono in punti (ciò che decide quanto è grande il testo).

## Conseguenze
- Elenco letto sul Mac di sviluppo (5K: 19 modalità, HiDPI comprese). Il cambio e il ripristino sono quelli di
  ADR 0061 (provati sul Mac con lo schermo virtuale); da provare con un monitor vero e su Windows.
- Sul Windows con ridimensionamento al 150–200 % abbassare la risoluzione ingrandisce meno del previsto: cambiare
  la scala da programma non è documentato, lasciato fuori.
- Due viewer sullo stesso computer: l'ultimo che sceglie vince; la risoluzione torna quando escono entrambi.
