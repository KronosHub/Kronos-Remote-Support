# ADR 0047 — Immagini negli appunti condivisi

**Data:** 2026-10-15
**Stato:** accettato

## Contesto
Uno screenshot fatto negli appunti (⌘⌃⇧4 sul Mac) non arrivava sull'altro computer: gli appunti condivisi
(ADR 0032) leggevano solo file e testo. Leggere un'immagine vuol dire decodificarla (un 5K in RGBA sono ~60 MB):
non si può fare a ogni controllo (ogni 500 ms) finché l'immagine resta negli appunti.

## Decisione
- Il thread degli appunti legge il contatore di sistema (`NSPasteboard.changeCount` su macOS,
  `GetClipboardSequenceNumber` su Windows) e rilegge gli appunti solo quando cambia; vale anche per testo e file.
- Ordine: file, testo, immagine. Un'immagine nuova viene codificata in PNG (crate `image`, la stessa versione di
  arboard), al massimo 40 milioni di pixel.
- Il PNG viaggia come un normale trasferimento di un file (`immagine.png`) sul canale `files`, con
  `Manifest.image = true`. Il ricevente lo decodifica (limite di memoria) e lo mette negli appunti come immagine.
- Un ricevente vecchio ignora il campo e riceve `immagine.png` come file da incollare: nessuna rottura.
- Linux: nessun contatore, quindi niente immagini (testo e file come prima).

## Conseguenze
- Barra di avanzamento, ✕ e limite dei 500 MB valgono anche per le immagini.
- Il PNG inviato sta in una cartella privata, cancellata alla copia successiva o a fine sessione.
