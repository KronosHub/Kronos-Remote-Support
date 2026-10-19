# 0055 — Monitor in stop: risvegliarlo invece di usare lo schermo virtuale

**Data:** 2026-10-19 · **Stato:** accettata (v1.8.3), corregge la 0048

## Problema
Collegandosi a un Mac con il monitor in stop (riposo dello schermo), si vedeva solo lo sfondo e non il campo della
password; bisognava disconnettersi e ricollegarsi. Dal log dell'host, a ogni primo collegamento: `no monitor:
sharing a virtual one` (1920×1080), al secondo il monitor vero (4608×2880). `no_monitor()` contava solo i display
*attivi* (`CGGetActiveDisplayList`): un monitor in stop non è attivo, quindi l'host lo trattava come un Mac senza
monitor e creava lo schermo virtuale della 0048. La schermata di accesso o di blocco però compare sul monitor vero,
che si accende subito dopo; lo schermo virtuale mostrava solo lo sfondo.

## Decisione
- `capture::virtual_display::monitors()` distingue tre casi, escluso lo schermo virtuale: `On` (almeno un monitor
  attivo), `Asleep` (collegato ma in stop: `CGGetOnlineDisplayList` lo elenca, quello degli attivi no), `None`.
- La sessione host crea `KeepAwake` prima di scegliere lo schermo e dichiara subito attività utente
  (`IOPMAssertionDeclareUserActivity`), che riaccende il monitor.
- Con `Asleep` la sessione aspetta fino a 5 s che il monitor diventi attivo; solo se resta spento (o con `None`)
  usa lo schermo virtuale.
- Se la sessione sta usando lo schermo virtuale e un monitor vero torna attivo (si risveglia tardi o viene
  collegato), al controllo ogni 5 s lo schermo virtuale viene lasciato: la sua cattura termina e riparte sullo
  schermo principale (ripresa della 0042).

## Conseguenze
- Collegandosi a un Mac con il monitor in stop si vede subito la schermata di accesso o di blocco.
- Il monitor del Mac controllato si accende a ogni collegamento (come succedeva già al primo input remoto).
- Se lo schermo virtuale è condiviso con un'altra sessione ancora aperta, resta finché quella non lo lascia.

## Verifica
Sul Mac (iMac, 4608×2880): `pmset displaysleepnow` → `monitors()` = `Asleep`; dopo
`IOPMAssertionDeclareUserActivity` → `On` in 20 ms. Clippy e fmt puliti. Da provare: collegamento dal portatile con il
monitor in stop.
