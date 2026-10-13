# 0013 — App desktop: istanza singola, finestra nascosta, icona di sistema

**Data:** 2026-10-07 · **Stato:** accettata

## Contesto
Durante i test due istanze con la stessa `device.key` si contendevano lo stesso ID; e chiudendo la finestra
l'app restava attiva senza poter mostrare il popup di conferma (richieste rifiutate in silenzio).

## Decisione
- `tauri-plugin-single-instance`: un secondo avvio porta in primo piano la finestra esistente.
- La X **nasconde** la finestra; l'app resta raggiungibile. Uscita da menu dell'icona di sistema ("Esci") o Cmd+Q.
- Icona nella barra di sistema (tray) con "Mostra" / "Esci"; clic sinistro mostra la finestra; su macOS anche clic sul Dock.
- Una richiesta di connessione riapre e mette a fuoco la finestra.
- Server: se un dispositivo si registra di nuovo, la connessione vecchia riceve `error{code: replaced}` e viene chiusa
  (prima restava aperta credendosi online pur essendo irraggiungibile).

## Dati locali
`app_data_dir` (macOS: `~/Library/Application Support/tech.kronos.desktopremoto/`): `device.key` (il server non è più configurabile, ADR 0017).
