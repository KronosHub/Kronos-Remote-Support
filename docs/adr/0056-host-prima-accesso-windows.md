# 0056 — Windows: raggiungibile dopo il riavvio, prima che qualcuno entri

**Data:** 2026-10-19 · **Stato:** accettata (v1.8.4), estende la 0033

## Problema
Dopo un riavvio di Windows il PC restava sulla schermata di accesso e non era raggiungibile: l'app (che tiene il
collegamento al server) parte solo all'accesso dell'utente ("Avvia all'accesso"). Il servizio SYSTEM della 0033
riparte da solo, ma fa solo cattura e input per conto dell'app.

## Decisione
- **Host prima dell'accesso**: quando nella sessione della console non c'è nessun utente, il servizio avvia la sua
  copia protetta dell'app come SYSTEM in quella sessione con `--logon-host <cartella>`. Niente finestre (la lista
  delle finestre della configurazione viene svuotata), barra, aggiornamenti, avvio automatico o istanza singola.
  Cattura e input diretti seguendo il desktop che riceve l'input (`windesk`), come l'agente.
- **Solo password personale**: davanti al PC non c'è nessuno che possa approvare; le richieste con la password
  temporanea vengono rifiutate subito. Nessun popup di chat.
- **Stesso ID**: il servizio copia dalla cartella dati dell'utente collegato (`%APPDATA%\tech.kronos.desktopremoto`,
  letta con il token dell'utente) `device.key`, `personal.json` e `prefs.json` in
  `Program Files\Kronos Agent\<servizio>\logon`, con DACL protetta solo SYSTEM e Administrators. Copia all'accesso e
  ogni 30 s mentre l'utente è collegato (una password personale nuova arriva così); se l'utente toglie la password
  personale, il file sparisce anche lì e l'host non parte.
- **Passaggio senza interruzione**: quando qualcuno entra in Windows (controllo ogni 2 s del nome utente della
  sessione) o l'app dell'utente si registra con lo stesso ID (il server manda `replaced` alla connessione vecchia),
  l'host si "ritira": lascia server e rete locale all'app dell'utente ma tiene aperte le sessioni in corso, che non
  passano dal server. Quando l'ultima finisce, esce. Il servizio non lo riavvia finché un utente è collegato.
- **Revisione del servizio**: il servizio esegue la copia dell'app fatta all'installazione, che gli aggiornamenti
  non toccano (ADR 0039: SYSTEM non esegue file modificabili dall'utente). `SERVICE_REVISION` (ora 1) è scritta
  dal servizio in `revision`; l'app che ne trova una più vecchia chiede una volta il permesso di amministratore per
  reinstallarlo, e mostra "Aggiorna" accanto al controllo delle finestre di amministratore.

## Conseguenze
- Serve il controllo delle finestre di amministratore attivo (il servizio) e una password personale.
- Dopo l'accesso, la sessione iniziata sulla schermata di accesso continua con l'host SYSTEM: il file manager vede
  le cartelle di SYSTEM, non quelle dell'utente, e il banner "schermo condiviso" non compare. Ricollegandosi si passa
  all'app dell'utente.
- Su un PC con più utenti, ID e password personale sono quelli dell'ultimo utente entrato.
- Non verificato su Windows reale al momento della scrittura (nessuna macchina virtuale disponibile); il modulo
  `agent` compila e passa clippy per Windows, il resto dell'app nei controlli su Windows di GitHub.

## E il Mac?
Provato sul Mac dell'utente (macOS 27.0.1) con un programma di prova firmato come l'app, avviato da launchd:
- un programma isolato non riceve i permessi dell'app; impacchettato come `.app` con lo stesso identificativo e la
  stessa firma, installato in un'altra cartella, sì (cattura e mouse nella sessione dell'utente);
- sulla schermata di login (agente `LoginWindow`, root) la cattura funziona e lo spostamento del puntatore anche, ma
  tasti e clic non arrivano al campo password.
Quindi sul Mac un host prima dell'accesso mostrerebbe la schermata di login senza poterla usare: non fatto. Con
FileVault attivo, inoltre, dopo un riavvio macOS non parte finché il disco non viene sbloccato. Per un Mac fisso la
via pratica è l'accesso automatico (con FileVault disattivato).

## Aggiornamento 1.8.5 — Desktop remoto
Una sessione utente lasciata scollegata da Desktop remoto non è la sessione della console: la console risulta senza
utente, e il servizio avrebbe avviato l'host prima dell'accesso con lo stesso ID dell'app ancora in esecuzione nella
sessione scollegata (si sarebbero tolti l'ID a vicenda sul server). Ora l'host parte solo se nessun utente è
collegato in nessuna sessione (`WTSEnumerateSessionsW`); `SERVICE_REVISION` 2. La sessione scollegata non ha schermo
né input: il viewer riceve un messaggio che lo spiega. Riportare in automatico la sessione sulla console non è stato
fatto.
