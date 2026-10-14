# 0045 — Nome utente, blocco dell'input del partner, pallino di stato

**Data:** 2026-10-14 · **Stato:** accettata

## Nome utente
- Ognuno imposta il proprio nome ("Il tuo nome", in `prefs.json`; Quick Support solo per la sessione).
- Viaggia nell'handshake di autenticazione (ADR 0012), **solo dopo che la password è verificata**: il viewer lo
  mette in `Proof`, l'host in `Accepted`. Così chi tenta password a caso non lo scopre, e la richiesta di accesso
  dell'host mostra già "Nome (ID)". Campi opzionali: le versioni precedenti li ignorano e non li mandano.
- `clean_name`: spazi compattati, caratteri di controllo tolti, al massimo 40 caratteri; applicato prima di inviare
  e all'arrivo (il nome viene dall'altra parte). Mostrato come testo (nessun HTML).
- Viewer: nome del partner prima del suo ID nella barra; host: "Nome (ID)" in richiesta, attività e banner.

## Blocco dell'input del partner
- `BlockInput { enabled }` (viewer → host), `InputBlocked { blocked, message }` (host → viewer: conferma, rifiuto
  o sblocco da parte di chi è davanti). Solo se il controllo remoto è attivo su quella sessione.
- `input::block::LocalBlock`, finché è tenuto:
  - macOS: event tap HID che scarta tastiera e mouse senza il marcatore `MARK` in `EVENT_SOURCE_USER_DATA`
    (impostato da enigo sui nostri eventi); riattivato ogni 200 ms se macOS lo disabilita.
  - Windows: hook `WH_KEYBOARD_LL` / `WH_MOUSE_LL` che scartano gli eventi senza `LLKHF_INJECTED` /
    `LLMHF_INJECTED` (l'input dell'agente SYSTEM e di SendInput passa). Ctrl+Alt+Canc resta sempre del sistema.
  - Linux: non supportato (rifiuto con messaggio).
- **Via d'uscita locale**: Esc premuto tre volte entro 2 s sblocca e avvisa il viewer. Il blocco termina anche con
  la sessione e con il processo (tap e hook gli appartengono). L'host mostra un avviso finché è attivo.

## Pallino di stato
- Accanto al tipo di connessione: verde, giallo se almeno una statistica è in attenzione, rosso se una è critica;
  stesse soglie dei contorni (latenza, CPU/RAM, qualità ridotta, resync); il tooltip elenca i motivi.
- Barra della finestra di controllo su due righe: nome/ID e pulsanti sopra, pallino, connessione e statistiche sotto.
