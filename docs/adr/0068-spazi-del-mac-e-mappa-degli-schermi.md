# 0068 — Spazi del Mac e mappa degli schermi del partner

**Data:** 2026-11-12 · **Stato:** accettata

## Contesto
Sul Mac ogni app a tutto schermo è uno Spazio: ci si sposta con Ctrl+← / Ctrl+→ e Ctrl+↑ apre Mission
Control. Dal computer che controlla quei tasti non arrivano mai al partner, perché li prende il sistema locale:
senza essere davanti al Mac non si può passare da un'app a tutto schermo all'altra. La vista "Tutti gli
schermi" (ADR 0029) esisteva già ma era una voce di un menu a tendina, senza la disposizione dei monitor.

## Decisione
- **Spazi:** tre pulsanti nella barra (`‹ Spazi ›`) e una riga nel pannello del pallino inviano al partner la
  combinazione come normali eventi `Key` (Ctrl giù, freccia giù, freccia su, Ctrl su): nessun messaggio nuovo,
  funzionano anche con un partner di versione precedente se si sa che è un Mac. Con un partner Windows gli stessi
  pulsanti fanno i desktop virtuali (Ctrl+Win+← →, Win+Tab). Un avviso al centro dello schermo dice cosa è stato
  inviato. Servono il controllo attivo e le scorciatoie di Mission Control attive sul Mac (lo sono di default).
  Provato su questo Mac con un evento simulato come quello di enigo (HID, flag Control): lo Spazio è cambiato e
  tornato indietro (id 121 → 91 → 121).
- **Sistema del partner:** `ControlMsg::Displays.os` (`std::env::consts::OS`), opzionale; i pulsanti compaiono
  solo se il partner lo dice (2.2.3+).
- **Mappa degli schermi:** `DisplayInfo` porta `x`, `y`, `area_width`, `area_height` (posizione e dimensione in
  coordinate del desktop: punti sul Mac, mentre `width`/`height` restano pixel nativi). Il menu a tendina diventa
  un pulsante che apre una mappa come nelle Impostazioni Monitor di macOS (solo il numero dentro ogni schermo,
  la barra dei menu sul principale), un elenco con nome e risoluzione e "Tutti gli schermi insieme". Da un partner
  vecchio (posizioni a 0) gli schermi sono disegnati affiancati. Nel pannello del pallino: `1 · 2 · 3 · Tutti`.

## Conseguenze
- Il protocollo resta compatibile nei due sensi (campi con `serde(default)`, test `displays_from_old_hosts`).
- Su Linux nessun pulsante degli Spazi (scorciatoie diverse per ogni ambiente desktop).

## Aggiornamento (2026-11-12, dopo la prova in produzione della 2.2.3)
Mission Control (Ctrl+↑) inviato dall'app non si apre sul Mac controllato, mentre Ctrl+← / Ctrl+→ cambiano
Spazio. Tolto il comando (e Win+Tab su Windows): nella barra al centro resta l'etichetta "Spazi", nel pannello
due pulsanti "Spazio precedente" / "Spazio successivo".
