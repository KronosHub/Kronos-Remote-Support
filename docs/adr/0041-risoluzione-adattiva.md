# 0041 — Risoluzione ridotta con poca banda

**Data:** 2026-10-14 · **Stato:** accettata

## Richiesta
Ridurre la risoluzione dello schermo trasmesso, solo quando la banda è scarsa (sviluppo previsto in ADR 0028).
Valutato e scartato lo sfondo nero durante la sessione: misurato con x264 a 1080p, risparmio ~30 % solo sui
keyframe e 3–25 % quando si spostano finestre, nulla nell'uso normale; su Windows richiede di cambiare lo sfondo
dell'utente (rischio di lasciarlo nero).

## Decisione
- **Dove**: nel loop di codifica dell'host (`stream::encode_loop`). La cattura resta alla dimensione piena; con
  risoluzione ridotta il frame viene scalato in CPU (`capture::scale::scale_bgra`) prima della codifica.
- **Quando** (`stream::rate::scale_for`), dal bitrate scelto dal regolatore (ADR 0028):
  sotto 0,8 Mbit/s → 75 %, sotto 0,45 Mbit/s → 50 % (stesse soglie dei gradini di fps). Si risale solo con il 25 %
  di margine sopra la soglia; tra due cambi almeno 2 s se si scende, 10 s se si sale: ogni cambio costa un keyframe.
- **Come**: a ogni cambio si crea un nuovo encoder alla nuova dimensione (inizia con un IDR con il nuovo SPS) e l'host
  invia `StreamStarted {width, height}`, come per il cambio di monitor. Il viewer riavvia il decoder solo se non ha già
  disegnato un frame della nuova dimensione. Le coordinate di input sono normalizzate: non cambiano.
- **Compatibilità**: i viewer precedenti gestiscono già `StreamStarted` con una dimensione diversa (riavvio decoder).

## Viewer
- Il canvas riempie la finestra (`width/height: 100%`, `object-fit: contain`): l'immagine si ingrandisce anche oltre la
  dimensione trasmessa. Le coordinate del mouse si calcolano sull'area dell'immagine, escluse le bande nere; un clic
  sulle bande non viene inviato.
- Statistiche della barra con larghezza fissa e valori allineati (font monospaziato).

## Aggiornamento 1.2.5 (2026-10-14)
- **Risoluzione massima** dello stream da 1920×1080 a 2560×1600: sugli schermi grandi e Retina il testo era
  ridotto anche con la rete buona (es. 2688×1680 punti trasmessi a 1728×1080).
- **Soglie per pixel** invece che per percentuale: sotto 1,5 Mbit/s al massimo 1920×1080 di pixel, sotto 0,8
  1440×810, sotto 0,45 960×540 (`rate::PIXEL_BUDGETS`); uno schermo 1080p si comporta come prima, uno 2560×1600
  scende al 71/53/35 %.
- **Computer lenti**: se la codifica (riduzione compresa) supera in media il 60 % dell'intervallo tra fotogrammi
  dopo 60 fotogrammi e l'immagine supera 1080p, la risoluzione resta limitata a 1080p per tutta la sessione.
- **Riduzione a media d'area** (`capture::scale::shrink_bgra`) invece che bilineare: ogni pixel sorgente conta, le
  linee sottili del testo non spariscono. La cattura Windows dallo schermo nativo resta bilineare (costo minore).
