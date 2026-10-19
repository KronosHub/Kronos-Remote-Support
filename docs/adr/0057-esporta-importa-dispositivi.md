# 0057 — Esportare e importare la lista dei dispositivi

**Data:** 2026-10-19 · **Stato:** accettata (v1.8.4)

## Richiesta
Poter esportare e importare la lista di "I miei dispositivi" (per esempio per passare a un altro computer).

## Decisione
- In **Organizza**: **Esporta** e **Importa**; con la lista vuota c'è subito **Importa**.
- Il file (`dispositivi-kronos.json` nella cartella Download, con un nome libero se esiste già) contiene dispositivi
  e cartelle. **Con le password salvate** (il segreto derivato di ogni dispositivo, che fa entrare senza conferma)
  tutto il contenuto è cifrato: PBKDF2-HMAC-SHA256 da 600 000 iterazioni con sale casuale, poi AES-256-GCM (crate
  `ring`, già usato dallo stack WebRTC); password di almeno 8 caratteri chiesta due volte. **Senza** password il file
  è JSON in chiaro con solo nomi, ID e cartelle.
- Import: il file si sceglie con il selettore di file della pagina; se è cifrato si chiede la password. Dispositivi
  e cartelle con lo stesso ID vengono sostituiti (il segreto già salvato resta se il file non ne ha), gli altri
  aggiunti: importare due volte lo stesso file non duplica nulla.
- Controlli sul file: formato e versione, al massimo 4 MB e 10 000 voci, ID di 9 cifre, nomi accorciati, numero di
  iterazioni entro un limite (un file danneggiato non blocca l'app).

## Conseguenze
- Niente plugin di dialogo nuovi: niente "Salva con nome", il file va sempre in Download.
- Il file con le password va custodito come le password stesse; senza la sua password non si legge.
