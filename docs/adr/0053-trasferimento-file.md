# 0053 — Trasferimento file con due pannelli

**Data:** 2026-10-16 · **Stato:** accettata

## Richiesta
Un pulsante "Trasferimento file" nella finestra della sessione che apre un'interfaccia divisa: le cartelle di chi
controlla da un lato, quelle del computer controllato dall'altro, entrambe aperte sulla home dell'utente.

## Decisione
- **Canale proprio** `fs`, creato da chi controlla insieme agli altri. Le app precedenti lo ignorano (le etichette
  sconosciute sono scartate), quindi nessuna incompatibilità: con un computer controllato vecchio la finestra dice
  di aggiornarne l'app (nessuna risposta in 10 s). Separato dal canale `files` degli appunti, che tiene solo l'ultima
  copia e mette i file negli appunti.
- **Protocollo** (`crates/stream/src/fs.rs`): messaggi JSON a pezzi (`j`…`J`: elenco di una cartella, nuova
  cartella, richiesta di file, annuncio dei file in arrivo, esito, annullamento) e i contenuti con gli stessi `C`/`D`
  degli appunti. Ogni richiesta ha un numero; una copia alla volta per direzione.
- **Dove finiscono i file** lo decide sempre chi li riceve: chi controlla sceglie la cartella locale (quella aperta),
  il computer controllato riceve la cartella di destinazione e controlla che esista. I percorsi dentro la copia sono
  validati come per gli appunti (niente `..`, assoluti o separatori).
- **Niente sovrascritture**: i file arrivano in una cartella nascosta `.kronos-in-…` dentro la destinazione e, a copia
  completa, vengono spostati con un nome libero ("nome (2).ext"). Copia interrotta o annullata: la cartella nascosta
  sparisce.
- **Limiti**: 100 000 elementi per copia, nessun limite di dimensione (gli appunti restano a 500 MB); elenchi fino a
  5000 elementi. I collegamenti simbolici si aprono nell'elenco ma non vengono copiati.
- **Chi è controllato vede cosa succede**: nell'Attività compaiono "sta sfogliando i file", "ha copiato N elementi
  (MB) da questo computer / in <cartella>", "ha creato la cartella". Non serve un permesso in più: chi controlla ha già
  mouse e tastiera dello stesso utente.
- **Interfaccia**: finestra `files-<sessione>` (componente `FileManager.svelte`), su, home, dischi (Windows),
  percorso modificabile, aggiorna, nuova cartella, file nascosti a scelta, selezione multipla (Ctrl/⌘, Maiusc),
  "Invia →" e "← Ricevi" verso la cartella aperta dall'altra parte, elenco delle copie con avanzamento e Annulla. Si
  chiude con la finestra della sessione. Niente trascinamento (su Windows il trascinamento di Tauri intercetta quello
  HTML), niente elimina/rinomina per ora.
- **macOS**: aprire Scrivania, Documenti o Download sul computer controllato richiede il permesso di accesso ai file
  (lo chiede macOS la prima volta); senza, l'errore spiega dove concederlo.

## Verifica
Test di `fs.rs` (elenco, file nascosti, messaggi a pezzi, nomi liberi, percorsi non validi, copie incomplete) e test
`crates/stream/tests/file_manager.rs` con due peer WebRTC reali in locale: elenco, nuova cartella, invio di una cartella
da 3 MB con avanzamento, ricezione due volte con nome nuovo, file mancante, annullamento senza residui. Interfaccia
controllata con dati finti in Chrome headless.

## Anche in questa versione
Il resoconto di fine sessione si chiude da solo dopo 30 secondi (come OK, con il conto alla rovescia sul pulsante).
