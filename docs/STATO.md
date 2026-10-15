# Stato del progetto

**Ultimo aggiornamento:** 2026-10-15
**Fase corrente:** v1.5.3 (le sessioni non cadono più quando il server di segnalazione perde un partner; ADR 0046) — v1.5.2 (✕ per interrompere i trasferimenti di file) — v1.5.1 (volume dell'audio, bitrate proporzionale ai pixel fino a 6 Mbit/s, keyframe ogni 60 s, niente pulsante Schermo intero) — v1.5.0 (barra di avanzamento dei file con velocità, chat che lampeggia e finestrella di chat sul computer controllato) — v1.4.0 (nome utente, blocco input del partner, pallino di stato, barra su due righe, chiusura delle sessioni mute; ADR 0045) — v1.3.0 (audio del computer controllato su Mac e Windows, pulsante HD, controllo aggiornamenti periodico; ADR 0044) — v1.2.5 (nitidezza: fino a 2560×1600, riduzione a media d'area; modificatori rilasciati; resync per minuto corretto) — v1.2.4 (clic sul Mac dopo il cambio di schermo, resync per minuto dinamico) — v1.2.3 (cattura che riparte dopo la schermata di accesso/blocco, ripetizione tasti, computer controllato sempre sveglio, statistiche colorate; ADR 0042, 0043) — v1.2.2 (risoluzione ridotta con poca banda, viewer a tutta finestra, statistiche a larghezza fissa; ADR 0041) — v1.2.1 (codice pubblico su `KronosHub/Kronos-Remote-Support`, ADR 0040) — v1.2.0 (revisione di sicurezza prima dell'apertura del codice, ADR 0039) — v1.1.1 (fix rete locale: tutti gli indirizzi, ripiego sul server, ID sessione compatibili con JS) — v1.1.0 barra laterale stile TeamViewer (ADR 0038) — v1.0.0 (chat tra host e guest, ADR 0037; prima release stabile) — v0.13.0 (nomi LAN, popup fine sessione, barra riordinata; ADR 0036) — v0.12.0 (signaling su Cloudflare, ADR 0035) — v0.11.0 (collegamento in rete locale senza server, ADR 0034) — v0.10.0 (agente Windows UAC/lock screen, ADR 0033) — v0.9.3 (installer Windows chiude l'app prima di aggiornare) — v0.9.2 (fix disattivazione password personale) — v0.9.1 cattura condivisa, fix Disconnetti — v0.9.0: finestra separata per sessione, appunti condivisi (testo e file), più controllori (ADR 0032)

## Fatto
- 1.5.3: `SessionClosed` dal server ignorato per le sessioni già avviate, tranne "peer left"; host: `PartnerLeft`
  per il messaggio giusto; connettersi a un dispositivo già aperto porta in primo piano la sua finestra (ADR 0046)
- 1.5.2: trasferimento dei file interrompibile da entrambi i lati (`TransferCancelled`: chi invia smette, chi
  riceve cancella i file parziali); 1.5.1 includeva anche "Modifica/Salva" per il nome utente
- 1.5.1: slider del volume (guadagno nella riproduzione, valore ricordato), bitrate massimo per dimensione
  (`rate::max_bitrate_for`, tetto 6 Mbit/s), keyframe periodico a 60 s, tolto "Schermo intero" (ADR 0041)
- 1.5.0: avanzamento dei trasferimenti di file (`files::ProgressReporter`, eventi `Transfer` in entrambe le finestre),
  pulsante Chat che lampeggia, finestra di chat flottante `chat-<session>` sul computer controllato (sempre in primo
  piano, in alto a destra, senza rubare il focus; cronologia tenuta dal backend). Da provare sul campo
- 1.4.0: nome utente nell'handshake (dopo la verifica della password), blocco dell'input del partner (tap HID macOS,
  hook LL Windows, Esc ×3 per riprenderlo), pallino di stato, barra a due righe, sessione chiusa dopo 20 s senza
  messaggi dal partner (ADR 0045). Da provare sul campo
- 1.3.0: audio dell'host (ScreenCaptureKit / loopback WASAPI → Opus 48 kHz 64 kbit/s → data channel non ordinato →
  riproduzione con coda di 60 ms), pulsante "Audio" spento di default e nascosto su Linux (ADR 0044); pulsante "HD"
  (off = massimo 1080p); aggiornamenti controllati all'apertura della sezione e ogni 4 ore. Da provare sul campo
- 1.2.5: stream fino a 2560×1600 con soglie per pixel e limite a 1080p sui computer lenti, riduzione a media
  d'area (ADR 0041, aggiornamento); il viewer rilascia ⌘⇧⌃⌥ non più premuti (scorciatoie locali come ⌘⇧4);
  orologio anche nelle finestre di controllo ("Resync per minuto" cala davvero)
- 1.2.4: sul Mac i clic restano validi dopo la schermata di blocco/accesso (enigo ricreato quando cambia l'area);
  "Resync per minuto" sempre visibile e calcolato sugli ultimi 60 s. Verificato dal log che la ripresa automatica
  della cattura della 1.2.3 funziona (display 205 → 206 sulla schermata di blocco → 207 dopo l'accesso)
- 1.2.3: la cattura riparte da sola se si ferma (Mac: display sostituito uscendo dalla finestra di accesso; Windows:
  errori della duplicazione allo sblocco), ripetizione dei tasti tenuti premuti, host sveglio durante la sessione,
  statistiche con contorno giallo/rosso e "Resync per minuto", puntatore a freccia (ADR 0042, 0043; da provare
  nell'app compilata)
- Risoluzione ridotta sotto 0,8/0,45 Mbit/s (75/50 %), viewer che riempie la finestra, statistiche fisse; changelog nell'app (ADR 0041)
- Worker Cloudflare con i limiti della revisione di sicurezza in produzione (verificato con due peer)
- Codice aperto (MIT): repository pubblico unico `KronosHub/Kronos-Remote-Support` con sorgente, CI e release; l'updater
  della 1.2.1 cerca prima lì e poi nel vecchio repository delle release (ADR 0040)
- Revisione di sicurezza e correzioni (ADR 0039): servizio Windows su copia protetta, password temporanea cambiata dopo 3 errori, limiti su rete locale/chat/Worker, appunti, pipeline fissate
- Barra laterale nella finestra principale: Supporto remoto, Dispositivi, Chat, Attività, Aggiornamenti (ADR 0038; da provare nell'app compilata)
- Signaling su Cloudflare Workers (gratuito), password con almeno 5 cifre e rinnovata a fine connessione (ADR 0035)
- Rete locale: scoperta mDNS + signaling diretto, collegamento senza server sulla stessa rete (ADR 0034)
- Agente Windows: controllo di finestre di amministratore (UAC) e schermata di blocco via servizio SYSTEM (ADR 0033); app installata verificata in VM
- Finestra separata per ogni sessione, appunti condivisi testo+file, più viewer sullo stesso host (ADR 0032)
- Kronos Quick Support: build `quick-access`, .exe / .zip / AppImage nelle release (ADR 0031)
- Barra viewer con traffico della sessione e CPU/RAM dell'app locale e del partner (ADR 0030)
- Vista "Tutti gli schermi": monitor composti in un'unica immagine lato host (ADR 0029)
- Qualità adattiva: bitrate/fps regolati dai report del viewer; fix buffer di riordino con perdite costanti (ADR 0028)
- Sessioni indipendenti dal signaling + keepalive WebSocket (v0.7.7)
- Requisiti e decisioni: ADR 0001–0009 in `docs/adr/`
- Toolchain: Rust 1.99 installato via rustup; ffmpeg/ffplay presenti (Homebrew)
- Workspace Cargo + git inizializzato (nessun commit ancora)
- `crates/capture`: trait `Capturer`, backend macOS ScreenCaptureKit, frame BGRA compatti
- `crates/codec`: `H264Encoder` (OpenH264), da BGRA ad Annex B
- `tools/capture-poc`: CLI che cattura, codifica, scrive `.h264` e stampa statistiche
- Test unitari: 54 ok (`cargo test --workspace`); clippy pulito
- Fase 2: signaling server (`apps/signaling`), `transport`, `peer-poc`; e2e locale ok (`scripts/e2e-local.sh`, RTT ~1 ms)
- App desktop `apps/desktop` (Tauri 2 + SvelteKit): ID, password, connessione, popup di conferma, test latenza, tray. DMG macOS in `target/release/bundle/dmg/`, installata in `/Applications`
- Repository GitHub privato iniziale; CI verde; installer v0.1.0 (pre-streaming) in release bozza
- Fase 3: streaming H.264 su traccia WebRTC + decodifica WebCodecs (ADR 0015), crate `protocol` e `stream`, fix VPN/ICE (ADR 0016), server non più configurabile (ADR 0017)
- Release bozza su GitHub: v0.1.0, v0.2.0 (streaming), v0.3.0 (controllo remoto + cattura Windows/Linux), v0.4.0 (multi-monitor)
- TURN coturn sulla VM, credenziali per sessione (ADR 0024; dalla v0.12.0 TURN opzionale consegnato dal Worker, ADR 0035); tipo di connessione nell'app; nome Kronos Remote Support (ADR 0025)
- Build/test con GitHub Actions ad avvio manuale (ADR 0026; ora tutto in `KronosHub/Kronos-Remote-Support`, ADR 0040)
- Aggiornamenti dall'app (updater Tauri firmato), release pubbliche su GitHub (ADR 0023; ora `KronosHub/Kronos-Remote-Support`)
- Streaming più robusto: buffer di riordino, keyframe limitati, niente frame skipping (ADR 0022)
- Accesso non presidiato con password personale, I miei dispositivi, avvio all'accesso (ADR 0021)
- Fase 4: crate `input` (mouse/tastiera, tasti fisici), cattura Windows DXGI e Linux X11, UI viewer con controllo (ADR 0018, 0019)
- Autenticazione: password + SPAKE2 + conferma host (ADR 0012), verificata end-to-end
- Server su VM: signaling + Caddy, systemd, certificato Let's Encrypt su `<ip-con-trattini>.sslip.io` (ADR 0011); dalla v0.12.0
  sostituito dal Worker Cloudflare `wss://kronos-signaling.kronos-signaling.workers.dev/ws` (ADR 0035)
- Test reali macOS: 1080p@30 ok (~17 ms/frame); Retina 3340x2160 solo ~11 fps (dettagli in DIARIO.md)

## Prossimo passo
- **Provare la 1.5.3** (entrambi i computer): sessioni lunghe senza cadute; se una cade, mandare il log di
  tutti e due i computer.
- **Provare la 1.4.0**: nome su entrambi i lati, blocco input (Mac e Windows, uscita con Esc ×3), pallino.
- **Provare la 1.3.0**: audio da Mac e da Windows (qualità, ritardo, sincronia con il video), pulsante HD; entrambi i
  computer vanno aggiornati (le versioni precedenti non aprono il canale audio).
- **Invio sulla schermata di accesso del Mac** (risolto, 2026-10-15): l'utente conferma che ora l'accesso da remoto
  funziona; probabilmente sistemato insieme alla sincronizzazione dei modificatori della 1.2.5.
- **Provare la 1.2.3 su Mac**: collegarsi a un Mac sulla finestra di accesso e sulla schermata di blocco, inserire la
  password (l'immagine deve riprendere in pochi secondi); tenere premuta una freccia nel Terminale; lasciare la
  sessione aperta oltre il tempo di stop del monitor. Poi le stesse prove su Windows.
- **Apertura del codice (in corso, ADR 0039)** — mancano: vedi "Da fare per l'open source" sotto.
0. Provare la 1.1.1 dall'aggiornamento: barra laterale; connessione in rete locale con conferma (prima risultava "rifiutata"); PC con VPN o macchine virtuali. Sui Mac controllare Privacy e sicurezza → Rete locale.
1. Verificare su host Windows (anche multi-monitor) e macOS reali le funzioni della 0.8.0: CPU/RAM/rete in alto; blocchi,
   risincronizzazioni, indicazione "qualità ridotta", voce "Tutti gli schermi". Se problemi: log (Windows `%LOCALAPPDATA%\tech.kronos.desktopremoto\logs`,
   Mac `~/Library/Logs/tech.kronos.desktopremoto`; righe `adaptive quality`).
2. Le molte risincronizzazioni viste su un Mac di prova possono dipendere dal bug del buffer di riordino corretto in 0.7.8: verificare.
3. Verificare Quick Support su Windows e Linux; incolla di file ricevuti in Finder/Explorer reali (0.9.0).
4. **Firma del codice** (rimandata, per ora nessun acquisto): Authenticode per Windows (WithSecure ha bloccato l'installer 0.8.0
   come sospetto) — Azure Trusted Signing o certificato OV/EV — e Apple Developer per macOS. Alternative gratuite:
   SignPath Foundation (progetti open source), Microsoft Store (MSIX firmato da Microsoft). Intanto: segnalazione falso positivo.
5. Migliorie: avvio cattura lento su monitor molto grandi; apertura più rapida in relay; dominio per nascondere l'IP;
   Apple Developer per eliminare l'avviso Gatekeeper.

## Da fare per l'open source (ADR 0039, 0040)
0. Archiviare il vecchio repository delle release quando non restano installazioni precedenti alla 1.2.1 (la 1.2.1 è
   pubblicata anche lì, solo `latest.json`).
1. SignPath Foundation (firma gratuita; policy e privacy già nel README) e segnalazione falso positivo a WithSecure.
2. Facoltativi: interruttore host per gli appunti (`allow_clipboard`), segreto personale cifrato con DPAPI su
   Windows, TURN di Cloudflare con credenziali temporanee, ID LAN mostrato come "non verificato", rimozione del
   servizio Windows alla disinstallazione.

## In coda (deciso con l'utente)
- **Microsoft Store** (gratuito): account Kronos su Partner Center (utente) + pagina informativa privacy; io: pacchetto
  MSIX in CI, avvio automatico via StartupTask, updater disattivato nella build Store. Distribuzione: link
  apps.microsoft.com / installer web firmato Microsoft / winget. I file GitHub restano non firmati (Quick Support escluso).

## Domande aperte
1. ~~VM Oracle~~ risolta: Oracle Linux 9.8 x86_64, 1 GB + 4 GB swap; non più usata dall'app dalla v0.12.0 (ADR 0035).
2. **Test Windows/Linux**: quali macchine sono disponibili per il porting della cattura?
3. ~~Repo GitHub~~ risolta: `KronosHub/Kronos-Remote-Support`, pubblico (ADR 0040).
