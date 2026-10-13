# Diario sessioni

## 2026-10-07 — Sessione 1: avvio progettazione
- Definiti requisiti: Windows/macOS/Linux, Rust + Tauri, progetto personale/studio, WebRTC con signaling/TURN propri.
- Creati: `CLAUDE.md`, `docs/ARCHITETTURA.md`, `docs/ROADMAP.md`, `docs/STATO.md`, ADR 0001–0003.
- Aperte 6 domande (vedi STATO.md).

## 2026-10-07 — Sessione 2: decisioni + Fase 1
- Confermate proposte: webrtc-rs, decodifica viewer in Rust, Svelte, ID 9 cifre, sviluppo su macOS. Server su VM Oracle Cloud dell'utente. ADR 0004–0009.
- Installato Rust 1.99 (rustup). Inizializzato git e workspace.
- Scritti `capture` (ScreenCaptureKit), `codec` (OpenH264), `capture-poc`.
- Problema risolto: binari crashavano con `Library not loaded: @rpath/libswift_Concurrency.dylib`
  (rpath del crate `screencapturekit` non si propaga). Fix: rpath `/usr/lib/swift` in `.cargo/config.toml`.
- Bloccato su: permesso Registrazione schermo da concedere al terminale.

### Primo test reale Fase 1 (MacBook arm64, display esterno 1920x1080)
- Display trovati: id 3 (1920x1080, scale 1.0), id 1 (3456x2234, scale 2.0). Default ha preso id 3 (primo in lista, non il principale).
- 10 s, schermo quasi statico: 47 frame (4.7 fps, solo frame cambiati), 1 keyframe,
  encode medio 20.05 ms, latenza cattura→bitstream 20.5 ms (max 32.8), 82.7 KiB (67 kbit/s).
- File riprodotto con ffplay: H.264 Constrained Baseline, yuv420p, 1920x1080.
- Rimossi warning OpenH264 (adaptive quantization / background detection non supportati per screen content).
- Valutazione: 1080p@30 regge (20 ms < 33 ms), margine basso. Retina nativo / 60 fps richiederanno VideoToolbox.

### Test sotto carico
| Scenario | Uscita | fps catturati / codificati | encode medio | latenza media (max) | bitrate |
|---|---|---|---|---|---|
| Display esterno, movimento | 1920x1080 | 28.0 / 26.5 | 16.9 ms | 18.0 ms (39.1) | 2236 kbit/s |
| Retina nativo, poco movimento | 3340x2160 | 12.1 / 10.7 | 42.4 ms | 62.6 ms (135.0) | 619 kbit/s |

- Retina a piena risoluzione (3456x2234) falliva: OpenH264 accetta al massimo 3840x2160 (verticale: 2160x3840).
  Fix: `CaptureConfig::max_size` (riquadro larghezza × altezza invece della sola larghezza), costanti
  `codec::MAX_WIDTH`/`MAX_HEIGHT`, errore `CodecError::TooLarge`. PoC: nuovo argomento `--max-height`; 0 = limite encoder.
- Conclusione: OpenH264 regge 1080p@30 (17 ms/frame). Vicino al 4K non arriva a 30 fps (42 ms/frame):
  serve VideoToolbox per risoluzioni alte. Default resta il riquadro 1920x1080.

## 2026-10-07 — Sessione 2 (cont.): Fase 2
- VM Oracle: Linux x86, solo IP. Decisione TLS: Caddy + `<ip>.sslip.io` (ADR 0010).
- webrtc-rs 0.21 ha API nuova (wrapper async su core sans-IO `rtc`): annotato in ADR 0004.
- Scritti: `crates/signaling-proto` (messaggi JSON, `PeerId` 9 cifre), `apps/signaling` (axum, hub con
  sessioni, persistenza ID hashati, rate limit), `crates/transport` (client WS + `PeerSession` con trickle ICE
  e buffer dei candidati arrivati prima della remote description), `tools/peer-poc`, `scripts/e2e-local.sh`.
- Test e2e locale: connessione WebRTC in ~30 ms dopo la creazione sessione, data channel aperto,
  RTT 0.6–1.8 ms. ID host stabile tra riavvii del server.
- Deploy preparato in `deploy/` (Dockerfile, compose, Caddyfile, README). Docker assente sul Mac: build non verificata.
- VM ispezionata (sola lettura): Oracle Linux 9.8 x86_64, ~410 MiB liberi, RustDesk attivo su 21115–21119,
  firewalld con solo ssh + porte RustDesk. Porte 80/443 dall'esterno: timeout (Security List OCI).
- Abbandonato Docker (RAM): deploy nativo con binario musl cross-compilato (cargo-zigbuild, 2.8 MB) +
  systemd + Caddy 2.11.7 (ADR 0011). Installati sul Mac: target musl, zig 0.17, cargo-zigbuild 0.23.4.
- Deploy eseguito con conferma utente: Caddy 2.11.7 (checksum ok), firewalld http/https, servizi `signaling` e
  `caddy` attivi e abilitati. Certificato Let's Encrypt per `<ip-con-trattini>.sslip.io` ottenuto al primo colpo.
  RustDesk non toccato, ancora attivo.
- Test dal Mac via `wss://<ip-con-trattini>.sslip.io/ws`: health ok, host + connect sullo stesso Mac, data channel
  aperto in 1.2 s, RTT 12–50 ms (anomalo rispetto a ~1 ms in locale: da indagare, forse coppia ICE non `host`).
- Memoria VM: signaling ~3 MB, Caddy ~15 MB (cgroup). Visto `dnf` a 520 MB RSS (non nostro; da tenere d'occhio con 1 GB).

## 2026-10-07 — Sessione 2 (cont.): app con GUI, password, conferma
- Richiesta utente: eseguibile con GUI e icona per Mac/Windows/Linux; poi password minuscola + popup di conferma.
- Creata `apps/desktop` (create-tauri-app, Tauri 2.12 + SvelteKit + Svelte 5). Backend: servizio sempre connesso
  al signaling con riconnessione, eventi `service` verso la UI, comandi `get_status/connect_to/new_password/
  respond_approval/set_server`. Logica ping/echo spostata in `transport::diag` (condivisa con `peer-poc`).
- Icona originale (monitor + doppia freccia, gradiente blu→verde acqua) da SVG con `tauri icon`.
- Nuovo crate `auth` (SPAKE2 + HMAC su fingerprint DTLS) e `transport::handshake`. ADR 0012.
  E2E: password errata rifiutata, giusta accettata. Test con l'app: popup mostrato, timeout = rifiuto, "Consenti" = ok.
- Bug trovati in test reale e corretti (ADR 0013):
  1. due istanze con stessa chiave → ID conteso; server ora chiude la connessione sostituita (`replaced`), app a istanza singola;
  2. finestra chiusa = popup invisibile → la X nasconde, tray con Mostra/Esci, richiesta riapre la finestra.
- La VM si è riavviata da sola alle 13:48 UTC (non per nostre azioni); servizi ripartiti automaticamente. Server ridistribuito.
- Windows: utente preferisce CI a cross-compilazione (licenza SDK Microsoft). Linux: CI. Workflow `ci.yml` e `release.yml`. ADR 0014.
- `gh` non autenticato: in attesa di login utente e conferma per creare repo.

## 2026-10-07 — Sessione 2 (cont.): repo GitHub, README, Fase 3
- Repository privato precedente creato (escluso `RUSTDESK/`), README in italiano. CI e release verdi:
  installer v0.1.0 per macOS arm64/x64, Windows (msi, exe), Linux (deb, rpm, AppImage) in release bozza.
- Fase 3 (streaming):
  - nuovi crate `protocol` (messaggi di controllo) e `stream` (pipeline host + logica sessione host/viewer);
  - `transport::video`: `VideoSender` (TrackLocalStaticSample H.264) e `FrameAssembler`/`VideoReceiver` (RTP → access unit, perdite → keyframe);
  - `capture::TestPattern` per test senza schermo; `peer-poc host --source screen|pattern`, `connect --save file.h264`;
  - e2e: 80 frame ricevuti e tutti decodificabili da ffprobe; schermo reale 1080p a 29,5 fps.
- RTT anomalo spiegato: ICE sceglieva a volte l'interfaccia Tailscale (`utun8`). Escluse interfacce tunnel (ADR 0016):
  RTT a regime 0,2 ms.
- App: viewer con canvas + WebCodecs (ADR 0015), toolbar (risoluzione, fps, RTT), schermo intero, Disconnetti;
  banner "schermo condiviso" con Termina lato host. Verificato: app viewer ↔ host CLI (pattern) 1280×720 a 30 fps.
- Richiesta utente: rimossi visualizzazione e cambio server dall'app (ADR 0017).
- App come host: senza permesso Registrazione schermo mostra errore guidato; macOS ha aperto le impostazioni.
  Firma ad-hoc ⇒ permesso da riconcedere a ogni build.

## 2026-10-07 — Sessione 2 (cont.): v0.2.0, Fase 4 e Windows/Linux
- Commit/push Fase 3, tag `v0.2.0` (release in build). Versione app 0.2.0.
- Permesso Registrazione schermo: l'interruttore attivato dall'utente era della build precedente (firma ad-hoc
  cambia a ogni build). Azzerata solo la voce dell'app con `tccutil reset ScreenCapture tech.kronos.desktopremoto`
  e rilanciata la richiesta. Uno screenshot ha catturato per errore un'altra finestra: cancellato subito; ora gli
  screenshot si fanno solo dopo aver verificato che l'app è in primo piano, e solo della zona necessaria.
- Richiesta utente: controllare ed essere controllati da Windows e Linux.
  - crate `input` (enigo + SendInput scancode), tabella tasti fisici per 3 OS, `InputSink` con rilascio tasti;
  - protocollo `Input`/`InputUnavailable`; UI: cattura mouse/rotella/tastiera, interruttore Controllo, "Solo visione";
  - cattura Windows (DXGI duplication) e Linux X11 (GetImage), scaler bilineare; macOS cattura il display principale;
  - verifica cross-target (`cargo clippy --target` Windows/Linux) e workflow `platform-smoke` (Xvfb/xdotool, Windows).
- Non provata l'iniezione sul Mac dello sviluppatore (avrebbe mosso il suo mouse e chiesto permessi al Terminale).
- `platform-smoke` su GitHub, problemi trovati e risolti:
  1. workflow: `cargo build -p capture-poc -p input --example smoke` compilava solo l'esempio;
  2. Linux: puntatore fermo al centro. Prima ipotesi sbagliata (scala RandR di enigo: corretta comunque, ora si usa la
     radice X11 e `XWarpPointer`); causa vera: Xvfb si resetta quando l'ultimo client si disconnette → `-noreset`;
  3. GitHub ha dato errori 500 sul push per qualche minuto (ritentato in automatico).
- Esito finale: Windows (DXGI + SendInput) e Linux X11 (GetImage + WarpPointer/XTest) verdi. Versione 0.3.0.

## 2026-10-07/08 — Sessione 2 (cont.): test reale utente, multi-monitor, v0.4.0
- Test utente (Mac + Windows in Parallels): niente controllo in nessuna direzione; da Windows un solo monitor del Mac.
- Causa del controllo mancante: sul Mac in `/Applications` c'era ancora una mia build locale **0.1.0** (pre-Fase 4),
  non la v0.3.0. Nessun bug trovato nel percorso input (smoke CI verdi).
- Implementata scelta del monitor (ADR 0020). Verificato sui due monitor del Mac via `peer-poc --switch-to`.
- v0.4.0: CI + platform-smoke verdi, release costruita (bozza). Sul Mac installata la build 0.4.0 locale e azzerate
  le voci TCC (Registrazione schermo, Accessibilità) della vecchia build con `tccutil reset ... tech.kronos.desktopremoto`.
- Connessione persa durante il lavoro; ripreso il 2026-10-08.

## 2026-10-08 — Sessione 2 (cont.): accesso non presidiato
- Richiesta utente: controllare i propri dispositivi senza conferma. Implementato con password personale (ADR 0021):
  `auth::derive_personal_secret` (PBKDF2), flag `personal` nell'handshake, `Access::Unattended` senza popup,
  `store.rs` (personal.json, devices.json, 0600), UI (imposta/cambia/disattiva, I miei dispositivi, Salva come),
  avvio all'accesso nascosto (`tauri-plugin-autostart`, `--hidden`). La finestra parte invisibile e viene mostrata
  in `setup` salvo avvio con `--hidden`.
- e2e esteso: password personale giusta (Unattended) e sbagliata (rifiutata). Bug di test: lo script leggeva la
  password prima che l'host finisse di derivare il segreto (PBKDF2 lento in debug) → ora attende la riga.
- Test: 51 ok.

## 2026-10-08 — Test utente v0.5.0 e streaming
- Utente: accesso non presidiato ok ("Perfetto"); streaming che a volte si blocca.
- Analisi e correzioni (ADR 0022): buffer di riordino per le ritrasmissioni NACK, richieste di keyframe ripetute,
  limite IDR sull'host, niente frame skipping, guardia sulla coda del decoder, contatore risincronizzazioni.
- Misura locale (pattern, 15 s): 450 frame a 30,1 fps, 0 risincronizzazioni. Test: 53 ok.

## 2026-10-08 — Aggiornamenti in app
- Installata la 0.5.1 sul Mac (build locale). La release v0.5.1 su GitHub era ancora in costruzione quando
  l'utente non l'ha trovata (bozza nel repo privato).
- Richiesta utente: aggiornamenti tramite pulsante. Scelte: repo pubblico separato per le release; ok a generare
  la chiave di firma e salvarla nei secret. Creato il repository delle release precedente, pubblico (solo README).
  Generata chiave updater (password casuale), secret impostati, backup in `~/.tauri`. Implementato updater (ADR 0023).
  Build locale firmata verificata (`.app.tar.gz` + `.sig`).
- In attesa: l'utente deve creare il token `RELEASES_TOKEN` (fine-grained) per pubblicare sul repo pubblico.

## 2026-10-08 — Fase 5 (TURN), nome Kronos, tipo di connessione
- v0.6.0: build ok ma creazione release nel repo pubblico fallita ("target_commitish invalid": tauri-action usava lo
  SHA del repo privato). Corretto con `releaseCommitish: main`; la 0.6.0 non è stata pubblicata, si passa alla 0.7.0.
- Richieste utente durante la Fase 5: tipo di connessione esplicito (diretta / relay); nome "Kronos Remote Support" e
  icona nei colori Kronos (ADR 0025).
- TURN (ADR 0024): credenziali HMAC nel signaling (verificate contro `openssl`), consegnate per sessione;
  `relay_only` in `PeerSession`, `peer-poc --relay-only`; `ConnectionPath` dalle statistiche ICE (bug: gli ID dei
  candidati nelle statistiche hanno un prefisso → confronto per suffisso). Script coturn pronti, non eseguiti.
- VM: nessun EPEL attivo, coturn da `oracle-epel-release-el9`; RAM disponibile ~615 MB.

## 2026-10-08 — Minuti Actions esauriti, build nel repo pubblico, coturn in produzione
- Job del repo privato non avviati: "recent account payments have failed or your spending limit needs to be
  increased" (minuti gratuiti finiti; macOS 10x). Scelta utente: build nel repo pubblico (ADR 0026).
  Deploy key SSH read-only sul repo privato, secret nel repo pubblico, workflow `checks.yml` e `release.yml`
  solo manuali; workflow del repo privato resi manuali.
- Utente ha creato le regole OCI (TCP/UDP 3478, UDP 49160–49200). Deploy: coturn 4.18.0 da Oracle EPEL attivo,
  signaling con "TURN relay enabled", 3478/TCP raggiungibile dal Mac. RustDesk invariato. RAM disponibile ~555 MB.
- Test `peer-poc --relay-only` via server di produzione: percorso Relay, 30 fps, 0 risincronizzazioni,
  RTT ~44 ms. Da migliorare: apertura in relay-only lenta (~6,5 s contro ~0,6 s diretta).
- Richiesta utente: verificare avvio automatico sulla VM. Tutti `enabled` (signaling, caddy, coturn, rustdesk-hbbs/hbbr,
  firewalld), firewall permanente, dipendenze da `network-online.target`. Corretto: Caddy non si riavviava dopo
  un crash (`Restart=no`) e coturn solo su abort → `Restart=on-failure` per entrambi (override coturn in setup-vm.sh).
- Richiesta utente: nascondere l'IP del server nell'app distribuita. Nessun dominio per ora; non vuole vederlo nei
  messaggi d'errore (screenshot: "connecting to signaling server wss://<ip-con-trattini>.sslip.io/ws ... refused",
  errori causati dal redeploy delle 09:09 UTC). Aggiunto `privacy.rs` (redact + messaggi amichevoli). v0.7.1.
- Riavvio di prova della VM: prima annullato dall'utente per errore, poi autorizzato ("se serve riavvia").
- Riavvio di prova VM (09:12 UTC): online in ~70 s; signaling (TURN abilitato), caddy, coturn, rustdesk-hbbs/hbbr,
  firewalld tutti attivi da soli; health HTTPS, TCP 3478 e 21116 raggiungibili dal Mac.
- Icona 0.7.2: K Kronos + monitor con due frecce (richiesta utente). Release 0.7.1 saltata (0.7.2 la include) per
  non avere due release in parallelo che si contendono "latest".
- Utente: la 0.7.0 inviata a un altro Mac non si apre ("spostala nel Cestino"): Gatekeeper + quarantena su app non
  notarizzata. Workaround `xattr -dr com.apple.quarantine` aggiunto al README pubblico. Soluzione definitiva:
  Apple Developer Program (Developer ID + notarizzazione in CI) — decisione dell'utente.

## 2026-10-08 — Bug "Connessione in corso" bloccata e controllo Windows → Mac
- Bug 1 (utente, screenshot Windows): password errata / accesso non presidiato non attivo → interfaccia ferma su
  "Connessione in corso…". Causa: l'host chiudeva subito la sessione dopo il rifiuto (messaggio "denied" perso) e
  il viewer aspettava fino a 45 s. Fix: host attende fino a 2 s (`linger`), il viewer interrompe l'attesa quando
  il server chiude la sessione, timeout 15 s con password personale, pulsante **Annulla**, rete di sicurezza 90 s.
- Bug 2 (utente): da Windows non controlla il Mac; da Mac controlla Windows senza problemi. Prova sul Mac con
  `peer-poc --input-test`: la build nuova non aveva alcun permesso (firma ad-hoc diversa a ogni build). Aggiunti:
  log su file (`~/Library/Logs/tech.kronos.desktopremoto/`), segnalazione sull'host di controllo attivo / errori di
  iniezione (`InputSink::health`, `HostEvent::ControlActive/ControlFailing`), riquadro **Permessi macOS mancanti**
  con pulsanti che aprono il pannello giusto (`CGPreflightScreenCaptureAccess`, `AXIsProcessTrusted`).
- Tolto il TURN su TCP dalla configurazione (webrtc-rs lo ignora: "Skipping unsupported non-UDP TURN url").
- Dopo che l'utente ha concesso i permessi: `peer-poc --input-test` verso l'app sul Mac → log "has the permission to
  simulate input" e "sta controllando mouse e tastiera". Controllo del Mac confermato. Nuovo monitor principale
  5760x3240: primo frame dopo ~9 s (avvio cattura lento, da migliorare). Release 0.7.4.

## 2026-10-08 — Firma stabile macOS e prova di durata
- Utente: blocco dopo qualche minuto controllando un altro Mac da Windows; permessi da riapprovare a ogni
  aggiornamento, da risolvere su tutti i Mac. Chiesti log e dettagli del blocco.
- Prova di durata locale (`peer-poc --input-flood`, 7 min, mouse a 60 Hz): 12 597 frame a 30 fps, 0 risinc.,
  RTT stabile. Non riproduce il blocco (non include cattura reale, iniezione reale, UI Windows).
- Firma stabile (ADR 0027): certificato autofirmato, requisito designato stabile verificato. Primo tentativo CI
  fallito ("failed to resolve signing identity"); release 0.7.5 parziale eliminata; import manuale nel workflow.
- Release 0.7.5 ricostruita: tutti i job ok; il pacchetto macOS di CI ha lo stesso requisito designato della build
  locale (`certificate root = H"27397ac3…"`). latest.json con darwin-aarch64/x86_64, linux, windows.

## 2026-10-08 — Blocco con la finestra Parallels in secondo piano
- Utente: su Windows (Parallels) il viewer si blocca quando la finestra perde il focus e si torna al Mac.
- Ipotesi: WebView2 rallenta la pagina in secondo piano mentre il backend continua a spedire 30 fps → coda
  illimitata di frame verso la UI, al ritorno l'arretrato blocca l'interfaccia. Inoltre una connessione fallita
  (es. VM in pausa) lasciava la vista congelata.
- Fix (v0.7.6): controllo di flusso backend↔UI (`frames_consumed`, max 15 frame non confermati, poi scarto e
  ripartenza da keyframe quando la UI ha smaltito); su `Failed` la vista si chiude / la condivisione termina.
- Prova sul Mac: 30 fps con il nuovo meccanismo; finestra nascosta 60 s → al ritorno 30 fps, RSS ~100 MB.

## 2026-10-08 — Causa vera dei blocchi: sessioni legate al signaling
- Log Windows (Parallels) inviato dall'utente: la WebSocket di signaling cadeva ogni 60 s esatti (solo da Windows/
  Parallels; il Mac resta connesso → NAT di Parallels che chiude connessioni TCP inattive). Ogni caduta chiudeva
  **tutte le sessioni attive** (`online()` le possedeva e le chiudeva all'uscita): sessione 13 aperta 10:59:54,
  chiusa 11:00:54; sessione 8 10:25:49 → 10:26:49. La vista restava sull'ultimo frame ("bloccata").
- Fix (v0.7.7): sessioni possedute da `run()`, sopravvivono alle riconnessioni del signaling; ping WebSocket ogni 20 s;
  su PeerConnection Closed/Failed la vista si chiude e la condivisione termina.
- Prova: app viewer + host di prova, `systemctl restart signaling` a metà sessione → app riconnessa in 3 s, stream
  continuato a 30 fps. (L'host di prova peer-poc termina quando perde il signaling: è il tool, non l'app.)
- Errori TURN 403 nel log: coturn rifiuta permessi verso IP privati (denied-peer-ip), atteso.

## 2026-10-08 — Qualità adattiva alla banda (v0.7.8)
- Utente: abbassare la qualità quando la banda scarseggia.
- Misure OpenH264: con QP minimo 0 il rate control ignorava il bitrate (3,3 Mbit/s per obiettivo 1 o 0,3);
  con skip attivo scende a 0 fps. Soluzione: QP minimo 10 + frame rate deciso da noi (ADR 0028).
- Viewer → host `report` ogni secondo (pacchetti ricevuti/persi, RTT); regolatore AIMD lato host; UI "qualità ridotta".
- Trovato e corretto bug del buffer di riordino: con perdita costante i buchi attendevano 200 ms uno dopo l'altro →
  ritardo crescente e perdite che continuavano dopo la fine della perdita reale.
- Prove: 15 % perdita simulata 12 s → 0,25 Mbit/s 10 fps, ritorno a 4 Mbit/s 16 s dopo; relay TURN reale pulito:
  nessuna variazione, 29 fps, 0 risinc.; e2e locale ok; test workspace ok.

## 2026-10-08 — Tutti gli schermi insieme (v0.7.9)
- Utente: vedere i monitor contemporaneamente. Scelta: composizione lato host in un'unica immagine (ADR 0029).
- `CompositeCapturer` (un thread per monitor), voce "Tutti gli schermi" nel selettore, tela fino a 3840×2160.
- Prova reale sul Mac a due monitor: tela 3840×1660, disposizione corretta, 0 risinc. `capture-poc --list` ora
  stampa anche posizione e dimensione in punti dei monitor.
- 0.7.8 (qualità adattiva) pubblicata: CI e release ok.

## 2026-10-08 — CPU, RAM e rete nel viewer (v0.8.0)
- 0.7.9 (tutti gli schermi) pubblicata: CI e release ok; installata sul Mac dello sviluppatore.
- Utente: mostrare CPU, RAM e rete della sessione accanto a fps. Fatto (ADR 0030): traffico da statistiche di
  trasporto WebRTC, CPU/RAM con `sysinfo` (processo + figli), il partner invia il proprio uso ogni 2 s.
- Prova locale: host di prova riporta CPU 2,4 % / RAM 53 MB; traffico 1374 KiB ricevuti ≈ byte video. e2e ok.
- Prossimo: client "Quick access" senza installazione, solo condivisione.

## 2026-10-08 — Kronos Quick Support (v0.8.1)
- 0.8.0 pubblicata. Utente: WithSecure su Windows blocca l'installer 0.8.0 come sospetto → spiegato (non firmato +
  comportamento da software remoto); opzioni: esclusione, falso positivo, firma Authenticode (decisione utente).
- Quick access (scelte utente: tre sistemi, solo password + conferma): feature `quick-access` + `tauri.quick.conf.json`,
  identità effimera, niente file, niente tray/updater/autostart/password personale, backend rifiuta connessioni in uscita.
- Prova macOS: bundle firmato 13 MB, registrazione ok, password personale rifiutata, UI ridotta, nessun file in ~/Library.
- `svelte-check` si è bloccato una volta senza CPU (processi appesi); rilanciato da solo: 0 errori.
- CI: job Quick nel `release.yml` pubblico (exe, zip, AppImage); README pubblico aggiornato.

## 2026-10-08 — Finestre separate, appunti condivisi, più controllori (v0.9.0)
- 0.8.1 pubblicata con i 4 file Quick. Check falliti per un test CPU su Linux (sysinfo salta processi a CPU zero al
  primo campione): corretto il test.
- Utente: viewer in finestra separata; appunti condivisi (poi: "non solo testo, ma anche file"); più persone possono
  controllare lo stesso computer. Fatto (ADR 0032).
- Test e2e appunti in CI su due display Xvfb: primo giro fallito (display :99 già in uso, sessione non pronta dopo
  6 s), poi scoperto il limite ~64 KiB per messaggio → testo ed elenchi file a pezzi. Ora verde.
- Prova macOS con app di debug (`KRS_TEST_CONNECT`): finestra separata ok; corretti barra di scorrimento e finestra
  non in primo piano.

## 2026-10-08 — Correzioni 0.9.1
- Utente: pulsante di chiusura della connessione non funziona; in due su una macchina Windows "errore API" (su Mac ok).
- Cause: manca il permesso `core:window:allow-close` per chiudere la finestra viewer da JS; DXGI non consente due
  duplicazioni dello stesso monitor nello stesso processo. Fix: permesso + `close_view` esplicito; cattura condivisa
  tra sessioni (`stream::shared_capture`). Prova macOS: due viewer su cattura reale ~27 fps, 0 risinc.

## 2026-10-09 — Password personale che restava attiva (v0.9.2)
- Utente: rimossa la password personale, ci si continuava a connettere senza conferma.
- Causa: su macOS la web view (wry 0.57) non implementa `window.confirm` → restituisce false → "Disattiva" (e
  "Rimuovi" dispositivo) non facevano nulla. Il backend era corretto (host rifiuta senza segreto personale).
- Fix: conferma interna all'app (`askConfirm`); rimuovendo/cambiando la password personale si chiudono anche le
  sessioni aperte con essa (`Shared.unattended`), e una sessione autenticata mentre la password cambiava viene rifiutata.
- Ambiente: il progetto è in ~/Documents sincronizzata con iCloud ("Ottimizza spazio"): sorgenti e node_modules erano
  `dataless` → letture bloccate (timeout di git e svelte-check). L'utente ha scaricato la cartella; `target/` (38 GB)
  è anch'essa in iCloud.

## 2026-10-09 — Firma, avvio senza Terminale, installer Windows (v0.9.3)
- Utente: niente acquisti per ora (né Apple Developer né firma Windows). macOS: con la firma stabile basta
  "Apri comunque" in Impostazioni → Privacy e sicurezza (README pubblico aggiornato, Terminale solo di riserva).
  Windows: opzioni gratuite proposte (Microsoft Store con firma MSIX gratuita, SignPath se open source, esclusione AV).
- Utente: installer Windows interrotto con "Failed to kill Kronos Remote Support" (PC con WithSecure). Il controllo
  standard di Tauri usa il Restart Manager (RmShutdown forzato) e abortisce se un processo non si lascia chiudere.
  Fix: hook `NSIS_HOOK_PREINSTALL/PREUNINSTALL` (`src-tauri/windows/hooks.nsh`) che chiude i processi dell'app per nome
  (`taskkill /F /T`) e attende fino a 10 s prima del controllo standard.

## 2026-10-09 — Agente Windows per UAC e schermata di blocco (ADR 0033, WIP)
- Utente: poter inserire password nei popup di amministratore (UAC) su Windows; in Quick, se non si concede admin,
  proporre di partire con permessi utente.
- Fatto su ramo `wip/windows-agent`: crate `windesk` (inseguimento desktop), `agent` (servizio SYSTEM + agente in
  sessione utente + pipe locale protetta + client cattura/input), integrazione in `stream` (via `agent_product`) e
  nell'app (toggle app completa, elevazione Quick, comandi UAC, UI).
- Verifica in VM Parallels (autorizzata): servizio → agente in sessione console; cattura di un popup UAC reale;
  iniezione puntatore coordinate corrette; verifica reciproca pipe (owner Administrators accettato oltre a SYSTEM).
  `tools/agent-poc` per i test. CI verde (test/linux/windows), agente compilato con MSVC.
- Da completare prima del rilascio 0.10.0: prova dell'app installata vera; desktop sicuro oscurato/lock screen live.

## 2026-10-09 — 0.10.0 rilasciata e verificata su app installata (VM)
- Merge su main, tag v0.10.0, release CI ok (tutte le piattaforme). Installer x64 provato su Windows 11 ARM64
  (emulazione): app installata, `--install-service` installa il servizio che avvia l'agente nella sessione console
  (log `agent starting session=1 name=\\.\pipe\kronos-agent-remote-1`). VM poi ripulita (servizio, app, attività).
- Da provare dall'utente: catena completa con un peer reale (viewer connesso → controllo su un popup UAC), desktop
  sicuro oscurato e schermata di blocco.

## 2026-10-09 — Verifica desktop sicuro; robustezza agente (v0.10.1)
- Utente: "non mi ha fatto cliccare sul popup UAC". Verificato in VM col desktop sicuro ATTIVO
  (PromptOnSecureDesktop=1, poi ripristinato): l'agente cattura l'UAC sul **desktop sicuro oscurato** e l'input vi
  arriva — Esc inviato dall'agente ha ANNULLATO l'UAC (consent.exe sparito, notepad admin non avviato, desktop
  tornato a Default; `SetThreadDesktop(Winlogon) ok` nel log di debug). Mouse: coordinate corrette. Il clic specifico
  non ri-testato per un blocco file nella VM (il servizio riapre l'agente), ma usa lo stesso percorso di iniezione.
- Causa probabile del problema utente: sul PC controllato l'interruttore UAC non era attivo (o agente non ancora
  avviato), quindi cattura ferma e clic persi. Robustezza (v0.10.1): `agent_available` passa da 1ms a 300ms di attesa,
  così un agente appena avviato non viene mancato (niente ripiego sul locale). Rimosso il log di debug da `windesk`.
- Nota: cattura/input scelgono l'agente all'inizio della sessione; attivare l'UAC va fatto PRIMA di connettersi
  (o riconnettersi dopo).

## 2026-10-09 — UAC opt-out e pulsanti in evidenza (v0.10.2)
- Utente: pulsanti UAC/riavvio-admin arancioni e vistosi; entrambi opt-out.
- Quick Support (Windows, non elevato): all'avvio tenta da solo l'elevazione (UAC). "No" → continua con permessi
  utente. Pulsante "Riavvia come amministratore" arancione con pulsazione se si è continuato come utente.
- App completa (Windows): all'avvio, se il servizio non è installato e non è stato rifiutato prima, propone di
  installarlo (UAC). Scelta ricordata in `prefs.json` (`uac_agent_declined`) per non richiedere a ogni avvio; il
  pulsante arancione "Attiva" lo abilita comunque. `agent_available` già a 300ms (0.10.1).

## 2026-10-09 — Rete locale senza server (v0.11.0) e valutazione RustDesk
- Utente: aggiungere la scoperta in rete locale; valutare i server RustDesk per le reti diverse.
- Fatto: `transport::lan` (mDNS + signaling TCP diretto), integrazione nell'app (LAN prima del server, ID memorizzato
  per l'uso offline, sezione "In rete locale"). Test reali: scoperta tra due istanze, handshake WebRTC completo solo
  via LAN (trovato e corretto bug dei byte persi dopo l'Hello).
- RustDesk: hbbs/hbbr parlano il protocollo proprietario di RustDesk (protobuf su TCP/UDP), non signaling WebRTC né
  TURN: non riutilizzabili senza riscrivere il trasporto. I server pubblici RustDesk sono per i client RustDesk. Il
  nostro server (signaling + coturn) è già sulla stessa VM: nessun risparmio a passare a RustDesk.

## 2026-10-09 — Passaggio a Cloudflare (v0.12.0)
- Utente: server Oracle non utilizzabile per sicurezza, serve soluzione gratuita. Scelto Cloudflare (account gratuito).
- Signaling su Worker + Durable Object, pubblicato e verificato con PC reali. Cloudflare TURN richiede carta: in
  attesa di un TURN gratuito con credenziali fisse (ExpressTURN) da salvare come segreti del Worker.
- Richieste utente: password con almeno 5 cifre; nuova password a fine connessione. Fatto.
- Al cambio di ID (nuovo server) la password personale viene disattivata con avviso.

- TURN configurato: segreti Worker TURN_URL (turn:free.expressturn.com:3478?transport=udp), TURN_USERNAME,
  TURN_PASSWORD (piano gratuito ExpressTURN). Verifica relay-only via Cloudflare: percorso Relay, 29 fps, RTT medio
  74 ms, 0 risinc.

## 2026-10-09 — Nomi LAN, popup fine sessione, barra riordinata (v0.13.0)
- Richieste utente: hostname + nome per i dispositivi LAN; popup di fine sessione con durata; Attività a scomparsa;
  barra del viewer ad aree. Fatto (ADR 0036). Prova su Mac: popup "Sessione con <peer> terminata" + durata; barra a
  riquadri (Rete/Risorse/Risinc./Video). Corretto: chiusura dal server lasciava la vista congelata senza popup.
- Prossimo (richiesto): chat tra host e guest, poi versione 1.0.

## 2026-10-09 — Chat e versione 1.0.0
- Aggiunta chat testuale host/guest sul canale di controllo cifrato (ADR 0037), con pannello e non letti nel viewer e
  nel banner di condivisione. Verificata in entrambe le direzioni con peer-poc. Con questa funzione si rilascia la 1.0.0.

## 2026-10-12 — Barra laterale (ADR 0038)
- Utente: compatibilità con TeamViewer QuickSupport → spiegato che non è possibile (protocollo chiuso, nessuna API
  per client terzi); proposta di avvio del TeamViewer ufficiale rifiutata.
- Fatto: barra laterale stile TeamViewer (Supporto remoto, Dispositivi, Chat, Attività; in fondo Aggiornamenti, stato,
  versione). Aggiornamenti diventa una pagina al posto del popup. Quick Support con barra a sole icone.
- Verificato: `svelte-check` 0 errori, anteprima browser con IPC simulato. Non ancora provato nell'app compilata.
- Nota: iCloud aveva tolto dal disco 1339 file di `node_modules`/`.svelte-kit` (`svelte-kit sync` bloccato);
  risolto con `brctl download`.

## 2026-10-12 — Fix rete locale (v1.1.1)
- Utente: su alcuni PC "connessione in rete locale fallita: No route to host (os error 65)"; e "accetto la richiesta
  ma risulta rifiutata".
- Cause: (1) ID sessione LAN con bit 63 (> 2^53): la UI li arrotondava, il "Consenti" rispondeva a una sessione
  inesistente → scadenza dopo 30 s → "l'utente remoto ha rifiutato". Ora bit 52. (2) mDNS annuncia tutti gli
  indirizzi (VPN, Parallels, IPv6 link-local) e il viewer usava solo il primo. Ora li prova tutti (IPv4 privati prima,
  link-local scartati, 2 s ciascuno). (3) Fallimento LAN senza ripiego: ora riprova tramite server. (4) macOS:
  aggiunto `Info.plist` con NSLocalNetworkUsageDescription e NSBonjourServices.
- Verifica: checks.yml verde sul ramo fix/lan (nuovi test: ordine indirizzi, ID < 2^53, salto di indirizzo
  irraggiungibile). Rust non è installato su questo Mac: test solo in CI.

## 2026-10-13 — Revisione di sicurezza (v1.2.0, ADR 0039)
- Utente: prima di rendere il codice open source (per SignPath), revisione per dati sensibili e sicurezza.
- Revisione in tre parti (dati sensibili, autenticazione/signaling, superficie d'attacco host). Nessun segreto mai
  committato; nei documenti ci sono IP, percorsi e dettagli dell'infrastruttura da togliere prima della pubblicazione.
- Corretto (ADR 0039): servizio SYSTEM che eseguiva un file sostituibile dall'utente (ora copia in Program Files, con
  migrazione automatica); log SYSTEM in ProgramData attaccabili con junction; blocco tentativi aggirabile in
  parallelo e usabile per bloccare tutti; rete locale senza limiti; panico su esadecimale non ASCII; `device.key`
  leggibile; chat senza limiti; file degli appunti scritti con la condivisione spenta e cartella `/tmp` condivisa;
  overflow nella somma delle dimensioni; Worker senza limiti; azioni GitHub non fissate.
- Rischi residui accettati e voci ancora da fare: in ADR 0039 e in STATO.md.
