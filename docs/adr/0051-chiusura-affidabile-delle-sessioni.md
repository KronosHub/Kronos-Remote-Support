# ADR 0051 — Chiusura affidabile delle sessioni

**Data:** 2026-10-15
**Stato:** accettato

## Contesto
L'utente ha premuto Disconnetti dal computer che controllava, ma quello controllato è rimasto in modalità
controllata finché l'app non è stata chiusa. La fine di una sessione arriva all'altro lato in due modi:
- `Bye` sul canale di controllo, spedito subito prima di chiudere la connessione, quindi può andare perso;
- `leave` al server, che lo inoltra come `session_closed: peer left`.

Dalla 1.5.3 (ADR 0046) una sessione avviata sopravvive ai ricollegamenti al server, ma il server, quando un
dispositivo si ricollegava, **cancellava la sessione**. Il `leave` successivo finiva in `unknown_session` e il
partner non veniva avvisato. I ricollegamenti sono frequenti: nel log di oggi del Mac ce ne sono due in una
sessione. Restava il controllo di silenzio da 20 s, che non scatta se dal partner arriva ancora qualcosa. I log del
computer rimasto appeso non sono disponibili.

## Decisione
- **Server** (Worker e server Rust): ricollegamento o caduta di un dispositivo avvisano il partner (`session_closed`
  come prima, ignorato dalle app ≥ 1.5.3) ma **non cancellano la sessione**: un `leave` successivo arriva. Righe
  limitate: Worker 32 per dispositivo (già in `connect`), server Rust 10 000 in tutto.
- **App**: `Bye` (Disconnetti, Termina, condivisione impossibile) aspetta fino a 1 s che il messaggio sia uscito
  (`outstanding_bytes` a zero) prima di chiudere la connessione.
- **OK sul resoconto** della sessione: comando `end_session`, che ferma visione o condivisione se ancora attive,
  chiude la connessione con `leave` al server (come `SessionDone`) e chiude la finestra `view-<session>`.

## Conseguenze
- Il Worker va pubblicato: il problema principale è lì, e la correzione vale anche per le app vecchie.
- Se un computer resta comunque appeso, servono i log di entrambi.
