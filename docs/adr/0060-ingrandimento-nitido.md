# 0060 — Ingrandimento nitido e dimensione dell'immagine nel viewer

**Data:** 2026-10-21 · **Stato:** accettata

## Contesto
Con uno schermo del viewer più grande di quello del computer controllato (es. 1366×768 su un 27" o un Retina)
l'immagine riempie la finestra (`object-fit: contain`, ADR 0041) e la ingrandisce il browser con un filtro
bilineare: il testo diventa grande ma sfocato. Studiate tre strade:
1. risoluzione del computer controllato adattata alla finestra (unica che aggiunge dettaglio vero; solo dove la
   risoluzione si può cambiare: VM, monitor virtuale, "solo da remoto") — rimandata, da descrivere in un ADR suo;
2. ingrandimento più nitido nel viewer;
3. modalità di dimensione dell'immagine.
L'utente ha scelto 2 e 3.

## Decisione
- **Renderer WebGL 2** (`lib/video-renderer.ts`): il frame decodificato (WebCodecs) va in una texture; il canvas
  ha la dimensione in pixel del dispositivo dell'area che copre e uno shader disegna l'immagine:
  - **bicubico Catmull-Rom** (16 letture `texelFetch`) quando si ingrandisce in modalità Adatta: bordi del testo
    più netti del bilineare, senza aloni forti;
  - **pixel più vicino** per i multipli interi e 1:1: ogni pixel del partner diventa un quadrato esatto.
  Se l'immagine è più piccola del frame, il canvas resta alla dimensione del frame e la riduce il browser (come
  prima, nessun aliasing dello shader). Se `texImage2D` non accetta un `VideoFrame` si passa da un canvas 2D.
  Senza WebGL 2 (verificato su un canvas di prova, perché un canvas tiene il primo tipo di contesto) si usa il
  contesto 2D di prima, con `image-rendering: pixelated` per Intero e Pixel reali.
- **Modalità** (menu nella barra del viewer, ricordata in `localStorage` `viewZoom`):
  - **Adatta** (predefinita): riempie la finestra mantenendo le proporzioni;
  - **Intero ×N**: il multiplo intero più grande che entra nella finestra, in pixel del dispositivo (almeno ×1);
  - **Pixel reali**: un pixel del partner per pixel dello schermo; barre di scorrimento se non entra, e senza
    controllo attivo la rotellina scorre l'immagine invece di arrivare al partner.
- **Dimensioni calcolate nello script** (`picture` in `+page.svelte`) dalla dimensione dell'area (ResizeObserver),
  dal `devicePixelRatio` (seguito con `matchMedia` quando la finestra cambia schermo) e dal frame dipinto. Il
  canvas ha esattamente la dimensione dell'immagine, centrato con margini automatici: le coordinate del mouse si
  calcolano sul rettangolo del canvas, senza più bande nere da escludere.

## Conseguenze
- Solo interfaccia: nessun cambio di protocollo, basta aggiornare il computer da cui si controlla.
- Il bicubico rende il testo più leggibile ma non aggiunge dettaglio; il colore resta a metà risoluzione
  (H.264 4:2:0), quindi il testo colorato su fondo colorato resta un po' impastato.
- Lo shader costa poco alla GPU del viewer (16 letture per pixel, solo quando arriva un frame o cambia la finestra).
