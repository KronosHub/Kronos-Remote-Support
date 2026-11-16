# 0071 — Nuova icona dell'app

**Data:** 2026-11-16 · **Stato:** accettata

## Contesto
L'icona di prima (K + monitor con frecce, bordo arancio spesso) era affollata e poco leggibile in piccolo. Proposte
in un canvas Design (https://claude.ai/artifact/9QP2mFnyp4qNSa3tWtvikN): quattro idee, poi varianti della "C"
(due schermi sovrapposti) e della C4 (monitor con piede).

## Decisione
Scelta la **C4.4** con un bordo arancio molto sottile: piastrella bianca squircle (rx 185 su 824, come le icone di
macOS e Windows 11), schermo dietro a contorno arancio, monitor davanti con cornice nera, schermo bianco e piede, K
nera con punta arancio. Sorgente unica `apps/desktop/src-tauri/icons/icon.svg` (copiata in `static/icon.svg`);
tutte le misure (`icon.icns`, `icon.ico`, PNG, `Square*Logo`) generate con
`npx tauri icon src-tauri/icons/icon.svg -o src-tauri/icons` (eliminare le cartelle `android/` e `ios/` che crea);
`static/favicon.png` = `32x32.png`. Quick Support usa le stesse icone.
