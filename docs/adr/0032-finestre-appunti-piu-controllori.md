# 0032 — Finestra separata per ogni sessione, appunti condivisi, più controllori

**Data:** 2026-10-08 · **Stato:** accettata

## Richieste
1. Lo schermo remoto si apre in una finestra separata.
2. Appunti condivisi tra i due computer: testo **e file**.
3. Un computer può essere controllato da più persone: la finestra con ID e password non deve dipendere da quella
   in cui si controlla.

## Decisioni
### Finestre
- Ogni sessione come viewer apre una finestra Tauri `view-<sessione>` (stessa pagina SvelteKit: la pagina sceglie
  app o viewer dall'etichetta della finestra, così gli stili restano condivisi). La finestra principale resta
  com'è (ID, password, approvazioni, banner "il tuo schermo è condiviso").
- Gli eventi del viewer prodotti prima che la finestra sia pronta vengono **accodati** (`ViewerCtl.backlog`, max 256)
  e restituiti da `attach_view`; poi arrivano normalmente. Chiudere la finestra termina la sessione; a sessione
  finita la finestra resta aperta con il motivo e il pulsante "Chiudi".
- Capability estesa a `view-*`. Hook di test solo debug: `KRS_TEST_CONNECT=<id>:<password>`.

### Più controllori
L'host gestiva già sessioni indipendenti (cattura, encoder e iniezione per sessione): su macOS funzionava, su
**Windows no** (0.9.0, segnalato dall'utente: "errore API" al secondo controllore) perché la duplicazione DXGI non
permette due duplicazioni dello stesso monitor nello stesso processo. Dalla 0.9.1 **una cattura per monitor è
condivisa** (`stream::shared_capture`): registro per configurazione di cattura, un thread che distribuisce i frame a
ogni sessione (canale da 2, chi resta indietro salta frame), ultimo frame consegnato a chi si aggiunge; la cattura si
ferma con l'ultima sessione. Ogni sessione mantiene il proprio encoder e la propria qualità adattiva. Verificato su
macOS con cattura reale: due viewer ~27 fps, 0 risinc.; test unitario con una cattura "esclusiva" simulata.

### Pulsante Disconnetti (0.9.1)
Non funzionava: la finestra si chiudeva da JavaScript senza il permesso `core:window:allow-close` (aggiunto); ora
termina anche la sessione esplicitamente (`close_view`) prima di chiudere.

### Appunti
- Thread dedicato (`stream::clipboard`, crate `arboard`) che possiede gli appunti (su X11 il proprietario deve restare
  vivo) e li controlla ogni 500 ms; ciò che scrive non viene rimandato indietro.
- Viewer → host `clipboard_mode {enabled}` all'avvio e al cambio (pulsante "Appunti condivisi" nella finestra,
  attivo di default). L'host condivide i propri appunti solo dopo `enabled: true`. Il viewer manda anche ciò che aveva
  copiato prima di connettersi; l'host no.
- Secondo data channel `files` (creato dal viewer accanto a `control`; `transport::sort_channels` li distingue): il
  traffico grosso non ritarda input e controllo. Messaggi binari: elenco file `m…M` (JSON a pezzi), contenuto `C`,
  fine `D`, testo `t…T`. Pezzi da 15 KiB, invio rallentato oltre 1 MiB in coda.
- **Limite scoperto dal test**: messaggi oltre ~64 KiB falliscono ("outbound packet larger than maximum message size"),
  quindi testo lungo (fino a 1 MiB) ed elenchi di file viaggiano a pezzi; sul canale di controllo solo testi ≤ 12 KiB
  (partner senza canale `files`).
- File: copiati subito al momento della copia (max 500 MB, 10 000 elementi; link simbolici esclusi), ricevuti in
  `$TMPDIR/kronos-clipboard/<pid>-<n>/<transfer>/` e messi negli appunti come elenco file (incolla in Finder/Explorer).
  La cartella è cancellata a fine sessione. Percorsi remoti validati (niente `..`, assoluti, `\`, `:`), dati oltre la
  dimensione annunciata rifiutati.
- Notifiche nella finestra del viewer e nell'Attività ("N elementi ricevuti negli appunti…").

## Verifica
- Unit test: file/cartelle annidate, 600 file (elenco a pezzi), testo multi-byte a pezzi, percorsi ostili, overflow.
- `scripts/e2e-clipboard-x11.sh` (CI, due display Xvfb con appunti separati): testo 100 KB viewer→host, file+cartella
  viewer→host con contenuto verificato, testo host→viewer.
- macOS: finestra separata con statistiche, pulsanti Appunti/Controllo, 30 fps (app di debug + host di prova).

## Limiti
- Incolla di file non provata su Finder/Explorer reali; su X11 il contenuto ricevuto sparisce a fine sessione.
- File copiati inviati subito (non su richiesta all'incolla): copiare cartelle enormi le invia comunque, entro i limiti.
