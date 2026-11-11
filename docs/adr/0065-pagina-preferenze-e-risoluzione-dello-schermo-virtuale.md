# 0065 — Pagina Preferenze, risoluzione fissa dello schermo virtuale, tema chiaro/scuro

**Data:** 2026-11-11 · **Stato:** accettata

## Contesto
L'utente vuole scegliere la risoluzione dello schermo virtuale che il Mac usato solo da remoto crea (ADR 0064),
e vuole una pagina **Preferenze** al posto di **Aggiornamenti**. Le impostazioni erano sparse nella scheda
"Consenti il controllo" di Supporto remoto. I mockup (canvas "Kronos Preferenze — mockup") proponevano A, una
pagina unica a sezioni, e B, una pagina a schede: l'utente ha scelto A e lo spostamento delle impostazioni.

## Decisione
- Il pulsante della barra laterale diventa **Preferenze** (icona a cursori, pallino se c'è una nuova versione).
  Le sezioni sono: Generale (nome, tema, avvio all'accesso), Chi si collega a questo computer può (appunti, file),
  Computer senza monitor ("solo da remoto", risoluzione dello schermo virtuale, monitor virtuale di Windows),
  Aggiornamenti (in cima quando c'è una nuova versione) e Assistenza (Scarica i log). Le righe hanno il testo a
  sinistra e il controllo a destra, con interruttori (`role="switch"`) al posto delle caselle.
- In Supporto remoto restano ID, password, permessi macOS, UAC e accesso non presidiato, con un collegamento alle
  Preferenze. Quick Support non ha Preferenze: lì nome e permessi restano nella scheda.
- **Risoluzione**: `Prefs.virtual_size` (`None` = Automatica). I valori sono `capture::virtual_display::SIZES`
  (le modalità dello schermo virtuale, letti con `stream::virtual_sizes()` e controllati dal comando
  `set_virtual_size`). La sessione la passa come `StreamConfig.virtual_size`. Quando lo schermo virtuale è
  condiviso, `ensure_screen` lo porta a quella misura con `capture::resolution::set`, anche se esisteva già, e
  aspetta che macOS la applichi. Con una misura fissa l'host non manda `ResolutionAdjustable` e ignora
  `MatchResolution`. Su un Mac con schermo integrato, o senza schermo virtuale, non cambia nulla.
- **Tema** Sistema / Chiaro / Scuro: `data-theme` su `<html>`. Le variabili scure valgono con
  `prefers-color-scheme: dark` solo se `data-theme` non è `light`, oppure quando è `dark`; c'è anche
  `color-scheme` per i controlli nativi. La scelta sta in `localStorage` (`theme`): vale per questo computer.
  L'evento `storage` la porta alle altre finestre dell'app.

## Conseguenze
- Una risoluzione fissa vale dalla connessione successiva. Chi controlla non vede più la modalità "Adatta la
  risoluzione".
- Il tema non passa per `prefs.json`: in Quick Support vale finché resta nel `localStorage` del webview.

## Aggiornamento (2.2.0)
- Dopo aver visto l'app, l'utente ha scelto la proposta **B**: Preferenze a schede (Generale, Connessioni, Schermo,
  Aggiornamenti, Assistenza). La risoluzione si sceglie da riquadri (`role="radio"`). Se c'è una nuova versione, il
  pallino compare sulla scheda e il pulsante della barra apre Aggiornamenti.
- **Schermata principale** rifatta come nel mockup: titolo della pagina, schede con icona, ID e password in un
  riquadro con pulsanti a icona, accesso non presidiato su una riga, selettore Con conferma / Password personale,
  Connetti a tutta larghezza e la striscia **I miei dispositivi** (3 salvati, prima quelli online, un clic per
  connettersi).
- `--accent-text` diventa scuro (`#111827`): il bianco sull'arancione aveva un contrasto di circa 2,6:1. La barra
  laterale resta bianca su arancione.
