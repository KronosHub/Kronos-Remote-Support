# 0063 — Seconda revisione di sicurezza e pulizia (2.1.0)

**Data:** 2026-10-29 · **Stato:** accettata

## Contesto
Analisi del codice della 2.0.1 su quattro fronti (rete e autenticazione, privilegi su Windows, canali dati e app,
pulizia), dopo quella della 1.2.0 (ADR 0039). Ogni problema è stato ricontrollato nel codice prima di correggerlo.

## Decisioni
**Rete e password**
- **Una sola negoziazione per sessione.** `transcript()` leggeva le impronte DTLS dall'SDP corrente, mentre DTLS
  tiene quella della prima negoziazione: un server di segnalazione (o chi sta sulla rete locale) poteva mandare a
  ciascun lato una nuova offerta con l'impronta dell'altro e mettersi in mezzo, con la password giusta.
  `PeerSession::set_remote` ora rifiuta una seconda descrizione remota; l'offerta va solo a chi risponde e la
  risposta solo a chi offre. L'app non rinegozia mai.
- **Password personale:** ogni tentativo conta come errato da quando inizia (`personal_attempt`) e smette di
  contare solo se è giusto (`personal_ok`), quindi tentativi in parallelo non superano i 5 in 10 minuti. Il controllo
  avviene prima di mandare `Accepted` (`approve` riceve anche i tentativi con la password personale): l'host non
  conferma più una password giusta arrivata dopo il blocco.
- **Password temporanea:** 3 errori in 10 minuti bloccano chi li fa (per ID); la password cambia dopo 10 errori in
  tutto. Prima 3 errori di chiunque la cambiavano: un estraneo con l'ID poteva impedire il collegamento a chi aveva
  la password giusta.
- **Sessioni non autenticate:** canali entro 20 s (`host_channels`), autenticazione (risposta dell'utente compresa)
  entro 45 s; poi la sessione si chiude. Prima restavano aperte per sempre.
- **Rete locale:** connessioni dirette solo da indirizzi privati, link-local, loopback e 100.64/10 (alcune VPN);
  al massimo 10 al minuto per indirizzo.
- **Video:** un fotogramma oltre 32 MB è scartato (un host poteva far crescere la memoria del viewer senza limite).

**Windows**
- **Prima dell'accesso** (host SYSTEM, ADR 0056): niente Trasferimento file né file negli appunti (`fs::refuse`,
  `allow_clipboard` spento): con la password personale chiunque, anche un utente standard, poteva scrivere in
  System32. Quando un utente accede, le sessioni continuano solo se è lo stesso utente da cui vengono ID e password
  (file `owner` scritto dal servizio a ogni copia, `only_owner_logged_on`): è il caso normale, in cui chi assiste
  inserisce la password di Windows di quell'utente. Se accede un altro utente si chiudono (`HostCommand::End`),
  altrimenti chi era collegato vedrebbe il suo desktop. Nella prima versione si chiudevano sempre: provato sulla VM,
  l'utente ha trovato scomodo doversi ricollegare (era il motivo della ADR 0056). ID e password continuano a venire
  da qualunque utente (scelta dell'utente).
- **Solo da remoto** (ADR 0059): impostazione del computer, file `remote-only` nella cartella del servizio (solo
  amministratori), con il nome dell'utente che l'ha attivata; si cambia con `--set-remote-only` elevato. Il servizio
  sposta sulla console solo la sessione di quell'utente. Prima era una preferenza di qualunque utente e spostava
  qualunque sessione scollegata, sbloccata. `SERVICE_REVISION` 4; chi l'aveva attiva la ritrova all'aggiornamento
  del servizio (`--install-service --remote-only`).
- **File dell'utente letti come l'utente** (`with_user_app_data`, `ImpersonateLoggedOnUser`), percorsi di rete
  rifiutati: la cartella dati viene dal registro dell'utente, che poteva farla puntare ai file di un altro utente o
  a una condivisione dove SYSTEM si sarebbe presentato con l'account del computer.
- **DLL solo da System32** (`/DEPENDENTLOADFLAG:0x800`) per l'app e `KronosVirtualMonitor.exe`, che chiedono
  l'elevazione e possono partire da Download.
- **`C:\VirtualDisplayDriver`** ricreata a ogni installazione con permessi propri (amministratori e SYSTEM;
  lettura per tutti): alla radice di C: qualunque utente può creare cartelle o collegamenti.
- **MSI dell'aggiornamento** tenuto aperto in sola lettura finché msiexec parte; `msiexec.exe` dal vero System32
  (`GetSystemDirectoryW`, non `%SystemRoot%`).

**Canali dati**
- **Download del Trasferimento file:** il viewer accetta solo elementi con i nomi chiesti (prima un host poteva
  aggiungere `.zshrc`, `.ssh` o altro nella cartella di destinazione, anche sovrascrivendo).
- **Nomi riservati di Windows** (`CON`, `NUL`, `COM1`…, punto o spazio finale, caratteri vietati) rifiutati.
- **Marchio "scaricato da Internet"** sui file ricevuti (`com.apple.quarantine`, `Zone.Identifier`).
- **Appunti:** spenti di default sul viewer e senza ciò che era copiato prima di accenderli. Sul computer controllato
  due opzioni (`deny_clipboard`, `deny_files` nelle preferenze): appunti condivisi e Trasferimento file.

**Pulizia**
- Tolti `apps/signaling` (i test end-to-end usano il Worker in locale con `wrangler dev`, `scripts/worker-dev.sh`),
  `tools/agent-poc`, i file della VM in `deploy/`, l'indirizzo di aggiornamento `albarth84` (non più pubblicabile:
  chi lo controllasse poteva servire una versione vecchia firmata), il comando `list_devices`.
- tokio: `signal` solo in `peer-poc`, `fs` solo in `stream`.

## Rischi residui accettati
- Il limite per destinazione del Worker (60 richieste al minuto, 10 per IP) si satura con almeno 6 IP.
- La password personale resta bloccabile da chi conosce l'ID (5 errori in 10 minuti): preferito alla sicurezza.
- Prima dell'autenticazione l'host dice ancora se l'accesso non presidiato è attivo (messaggio utile all'utente).
- Ogni finestra dell'app può chiamare tutti i comandi (nessuna XSS trovata); NASM nel workflow di release senza
  controllo della versione; binari Windows senza firma Authenticode (ADR 0039).
- Chi conosce la password personale di un utente standard può ancora usare la schermata di accesso di Windows.
