# 0077 — Il cursore del viewer prende la forma di quello del partner

**Data:** 2026-11-24 · **Stato:** accettata

## Contesto
Sopra il video il viewer mostrava sempre la propria freccia. Con un partner Windows il puntatore non è nei
fotogrammi (ADR 0019), quindi non si capiva se si era sopra un testo (cursore a I), un link (manina), un bordo da
trascinare o un'attesa. Con un partner Mac il puntatore è disegnato nel video ma arriva in ritardo.

## Decisione
- **L'host guarda la forma del puntatore** ogni 100 ms su un thread suo (`stream::cursor`) e manda solo i
  cambiamenti con il nuovo messaggio `ControlMsg::Cursor` (i viewer vecchi lo ignorano):
  - **Windows**: `GetCursorInfo`. I cursori di sistema (handle condivisi di `LoadCursorW(IDC_*)`) diventano parole
    chiave CSS (`default`, `text`, `pointer`, `wait`, `progress`, `crosshair`, `move`, `not-allowed`, `help`, i
    quattro `*-resize`): il viewer mostra i propri cursori nativi, nitidi. Gli altri (cursori delle applicazioni)
    viaggiano come PNG: il cursore disegnato su nero e su bianco con `DrawIconEx` dà colore e opacità; i pixel che
    invertono lo sfondo escono neri. Scala = DPI di sistema / 96. Cursore nascosto: `none`.
  - **macOS**: `NSCursor.currentSystemCursor` (deprecata ma funzionante su macOS 27), sempre come PNG 2x. Per non
    convertire l'immagine 10 volte al secondo confronta dimensioni e punto caldo, e l'immagine intera una volta al
    secondo.
  - **Linux**: non ancora (il viewer tiene la freccia).
- **Il viewer** trasforma il messaggio in un valore CSS (`session::cursor_css`: solo parole chiave o PNG in base64
  validati, punto caldo in punti, `-webkit-image-set(… Nx) x y, default`) e lo applica al canvas mentre controlla.
  Verificato che WebKit lo accetta.
- Sulla schermata di sicurezza di Windows (Ctrl+Alt+Canc, UAC, blocco) l'app non vede il puntatore: se il desktop
  di input non si apre o non è "Default" (`OpenInputDesktop`), freccia.

## Conseguenze
- Serve la nuova versione su entrambi i computer.
- Con un partner Mac il puntatore resta anche nel video (come prima): due cursori sovrapposti, quello locale con la
  forma giusta e senza ritardo.
- Prova: `cargo run -p stream --example cursor_probe -- <cartella> [secondi]` stampa i cambiamenti e salva le
  immagini; su Windows disegna anche tutti i cursori di sistema (provato sulla VM, sessione utente).
