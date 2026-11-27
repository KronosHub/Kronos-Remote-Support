# 0078 — Modalità testo: il comando `kronos-remote`

**Data:** 2026-11-27 · **Stato:** accettata (v2.4.0)

## Contesto
L'utente vuole usare l'app anche da terminale: con un comando vedere ID e password del computer e confermare in
modo testuale chi si collega. Se serve, lo stesso comando deve proporre il monitor virtuale.

## Decisione (scelte dell'utente: nome, aggancio all'app, pulsante in Preferenze)
- **Nome**: `kronos-remote`. È uno script che avvia l'eseguibile dell'app con `--cli`; la modalità testo parte
  all'inizio di `run()`, prima di Tauri: niente finestre, icona nel Dock o istanza singola.
- **Si aggancia all'app**: l'app (finestra aperta o nascosta nella barra, anche Quick Support; non l'host prima
  dell'accesso) ascolta su una porta TCP di `127.0.0.1` scelta dal sistema e scrive porta e un token casuale in
  `cli.json` nella sua cartella dati (leggibile solo dall'utente, `store::private_file`). Il comando legge il file,
  manda il token come prima riga e poi parla a righe JSON: l'app manda `hello` (stato, richieste in attesa,
  sessioni in corso, numero di schermi, monitor virtuale, `remote_only`, sistema) e poi gli eventi `status`,
  `approval_request`/`approval_closed`, `hosting_started`/`hosting_ended`, `log`; il comando manda `approve`,
  `new_password`, `stop_hosting`, `virtual_monitor`, `remote_only`.
- **App chiusa**: il comando la avvia nascosta con `--hidden --cli-owned` (su macOS/Linux in un gruppo di processi
  suo, così Ctrl+C chiude il comando e l'app esce da sola con le sue pulizie) e si collega appena c'è `cli.json`.
  L'app avviata così esce quando l'ultimo terminale si scollega, a meno che nel frattempo qualcuno ne abbia aperto
  la finestra.
- **Conferme nel terminale**: con un terminale collegato la finestra principale non viene mostrata per la
  richiesta di connessione, l'accesso non presidiato e il resoconto di fine sessione; il popup resta comunque nella
  finestra, e vale la prima risposta.
- **Comandi**: `s`/`n` per la richiesta più vecchia, `p` nuova password, `i` di nuovo ID e password, `d`
  disconnetti, `m` monitor virtuale, `q` esci.
- **Monitor virtuale**: su Windows, se l'app lo porta e non ci sono schermi (`capture::displays` vuoto), il comando
  chiede subito se installarlo (stesso programma della 0049, con la conferma di amministratore); `m` lo installa o
  lo rimuove. Sul Mac non c'è nulla da installare (lo schermo virtuale nasce da solo, ADR 0048): `m`, e la domanda
  all'avvio se non ci sono schermi, accendono o spengono "Computer usato solo da remoto", che lo rende lo schermo
  principale (ADR 0064).
- **Installazione dalle Preferenze** (Generali › Comando da terminale, non in Quick Support): macOS
  `/usr/local/bin/kronos-remote` (se la cartella non è scrivibile, copia con la password di amministratore via
  `osascript`; rifiutato se l'app gira da AppTranslocation), Linux `~/.local/bin/kronos-remote`, Windows
  `%LOCALAPPDATA%\tech.kronos.desktopremoto\bin\kronos-remote.cmd` con la cartella aggiunta al PATH dell'utente.
  "Installato" vuol dire che lo script avvia proprio questa copia dell'app.
- **Windows**: l'app è un programma grafico; in modalità testo si aggancia alla console da cui è partita
  (`AttachConsole`, altrimenti `AllocConsole`) e apre `CONIN$`/`CONOUT$` se mancano. Lo script `.cmd` fa sì che il
  terminale aspetti la fine del comando.

## Conseguenze
- Un'app più vecchia della 2.4.0 aperta non ha il server: il comando prova ad avviarne un'altra, che passa la mano
  a quella aperta, e dopo 30 s dice di chiuderla.
- Il token protegge da altri utenti dello stesso computer, non da programmi dello stesso utente (che potrebbero
  comunque leggere la chiave del dispositivo).
- Linux senza interfaccia grafica: non supportato (l'app avviata ha bisogno di un display).
- Provato sul Mac con due copie di prova (identificativi diversi): avvio dell'app, ID e password, richiesta
  accettata e rifiutata, nuova password, disconnetti, `m` sul Mac, uscita dell'app avviata dal comando, aggancio
  a un'app già aperta. Windows e l'installazione del comando: da provare.

## Riavvio e schermata di accesso (verificato il 2026-11-27)
- **Windows** (ADR 0056): sulla VM, con il servizio installato e la password personale, dopo un riavvio l'host
  prima dell'accesso è in linea in 1 s con lo stesso ID; con "Avvia all'accesso" l'app dell'utente lo sostituisce
  ~25 s dopo l'accesso. Con l'utente disconnesso (nessuno collegato, schermata di accesso) l'host riparte nella
  nuova sessione della console e si registra in ~30 s. Senza "Avvia all'accesso" dopo l'accesso il computer resta
  non raggiungibile: l'host si ritira all'accesso e nessuno prende il suo posto.
- **macOS**: invariato rispetto alla 0056 ("E il Mac?"): sulla schermata di login cattura e puntatore funzionano,
  tastiera e clic no; con FileVault attivo (come sul Mac dell'utente) dopo un riavvio il Mac non va in rete finché
  il disco non viene sbloccato.
