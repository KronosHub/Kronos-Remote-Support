# 0064 — Schermo virtuale principale sul Mac usato solo da remoto; log scaricabili

**Data:** 2026-11-11 · **Stato:** accettata

## Contesto
Mac mini M2 (2023, macOS 26.3) senza niente collegato, con "Computer usato solo da remoto": il viewer vedeva il
desktop ridotto a un quarto in alto a sinistra, tutto verde (verde scuro il resto) e il mouse non seguiva il
puntatore. Il log dell'host mostrava `display=3` 1920×1080 catturato con VideoToolbox e nessuno schermo virtuale:
macOS tiene un display di ripiego che conta come monitor vero (`monitors()` = `On`, ADR 0055), quindi lo schermo
virtuale dell'ADR 0048 non veniva creato. Non riprodotto qui (Mac con monitor): lo schermo virtuale, catturato e
codificato allo stesso modo, esce corretto. Non è stato possibile fare altre prove sul Mac mini.

## Decisione
- Con `remote_only` (`StreamConfig.resize_display`) su un Mac **senza schermo integrato attivo** (Mac mini,
  Studio, Pro; mai un MacBook o un iMac, dove può esserci qualcuno davanti) la sessione crea sempre lo schermo
  virtuale e lo rende **principale con gli altri schermi in duplicazione** (`VirtualDisplay::take_over`:
  `CGConfigureDisplayMirrorOfDisplay` verso lo schermo virtuale e origine (0, 0), `kCGConfigureForSession`).
  Desktop, barra dei menu, Dock e mouse passano lì. Se la duplicazione è rifiutata, lo schermo virtuale diventa
  almeno quello principale. Alla fine (ultimo `Arc` rilasciato) la duplicazione viene tolta; al più tardi la
  annulla il logout.
- Lo schermo virtuale resta per tutta la sessione: `monitor_back` non lo toglie con `remote_only`.
- Lo schermo principale per la cattura è `CGMainDisplayID`, non più quello con origine (0, 0): con la duplicazione
  anche gli schermi duplicati stanno in (0, 0).
- All'inizio di ogni condivisione sul Mac il log riporta i dati CoreGraphics di ogni schermo (produttore, modello,
  seriale, integrato, principale, attivo, in stop, duplicazione, punti e pixel): per capire un Mac visto solo da
  remoto.
- **Scarica i log** nella pagina Aggiornamenti (l'app non ha una pagina Impostazioni): zip nella cartella Download
  con i log dell'app (e su Windows del servizio, se leggibili), al massimo gli ultimi 32 MB per file, e `info.txt`
  con versione e sistema. Crate `zip` senza funzioni predefinite, compressione con `flate2` (già presenti).
- `capture-poc`: `--cg` (dati CoreGraphics), `--virtual-display`, `--take-over`, `--copy` per le prove.

## Conseguenze
- Provato qui: con `--take-over` i due schermi del Mac passano in duplicazione sullo schermo virtuale, la cattura
  (VideoToolbox, senza copie) mostra tutto il desktop, alla fine la disposizione torna identica.
- Sul Mac mini resta da provare; se il display di ripiego non si lasciasse duplicare, resta affiancato.
- Senza `remote_only` un Mac senza monitor con il display di ripiego si comporta come prima: i dati nel log diranno
  se si può riconoscere e trattare allo stesso modo.
- Lo schermo del Mac, se qualcuno lo collegasse durante la sessione, mostra la stessa immagine dello schermo
  virtuale (1920×1080 o la risoluzione adattata).
