# ADR 0048 — Schermo virtuale sul Mac senza monitor; sessione aperta senza schermo

**Data:** 2026-10-15
**Stato:** accettato; un monitor in stop non conta più come "senza monitor" (ADR 0055)

## Contesto
Collegandosi a un PC Windows o a un Mac senza monitor, la sessione si chiudeva dopo ~2 s: la cattura non trova
nessuno schermo (`no displays available`), l'host manda `StreamError` e il viewer chiude. Le soluzioni senza
software sono gli adattatori HDMI "dummy plug".

## Decisione
**Sessione senza schermo (tutti i sistemi).** Se la cattura non parte all'inizio, l'host non chiude: manda
`ScreenUnavailable { message }` (nuovo messaggio), resta in ascolto (ping, chat, appunti, comandi) e riprova
ogni 5 s; quando ci riesce manda `StreamStarted` come dopo un riavvio della cattura. Il viewer mostra il
messaggio al posto di "In attesa dello schermo" e lo toglie al primo `StreamStarted`. Un viewer vecchio ignora il
messaggio e resta in attesa (non chiude).

**Schermo virtuale (macOS).** Se CoreGraphics non vede nessuno schermo attivo (o ScreenCaptureKit non ne
elenca), l'host crea un monitor virtuale 1920×1080 a 60 Hz con `CGVirtualDisplay` (API privata di CoreGraphics,
macOS 11+, usata da DeskPad e BetterDisplay) e lo cattura come un monitor normale. Ogni selettore è verificato con
`respondsToSelector:` prima della chiamata: se Apple la cambia, si torna alla sessione senza schermo invece di
andare in crash. Lo schermo è condiviso tra le sessioni (`virtual_display::shared`) ed è tolto quando finisce
l'ultima. Provato su questo Mac: compare con un id nuovo e sparisce al rilascio.

**Windows.** Uno schermo virtuale richiede un driver (Indirect Display Driver) firmato da installare con l'app:
rimandato a dopo la firma del codice (SignPath). Intanto: sessione aperta senza schermo, o dummy plug.

## Conseguenze
- API privata: un aggiornamento di macOS potrebbe toglierla; il ripiego è la sessione senza schermo.
- La cattura di uno schermo virtuale senza monitor fisico va verificata su un Mac davvero senza monitor.
