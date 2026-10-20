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

## 2026-10-13 — Repository pubblico (v1.2.1, ADR 0040)
- Utente: spostare tutto sull'account GitHub aziendale e procedere con apertura del codice e SignPath.
- Documenti ripuliti (IP e chiavi della vecchia VM, percorsi, ID dispositivi, nomi degli account), aggiunto `LICENSE`
  (MIT, Kronos Informatica), sezioni README su firma del codice e privacy.
- Nuovo repository `KronosHub/Kronos-Remote-Support` creato da un solo commit (la cronologia resta nel repository
  privato). Workflow `checks.yml` (push, pull request, manuale) e `release.yml` nel repository stesso, con il
  `GITHUB_TOKEN`: niente più deploy key e token separato. Secret di firma caricati; checks verdi.
- 1.2.1: l'updater cerca prima nel nuovo repository e poi nel vecchio.
- Worker Cloudflare aggiornato in produzione e verificato con due peer (il primo `wrangler deploy` era finito su un
  account sbagliato: copia di prova da eliminare).
- Release 1.2.1: le build macOS si sono fermate su "Resource not accessible by integration" nel creare la release
  (`releaseCommitish` uguale al tag); release creata a mano e job rilanciati, ora `releaseCommitish` non è più
  impostato. `latest.json` con le note dal nuovo `CHANGELOG.md`, copiato come release del vecchio repository:
  le installazioni fino alla 1.2.0 trovano la 1.2.1 (verificato).
- Changelog nell'app (richiesta utente): `CHANGELOG.md` → note della release → l'app mostra solo le versioni più
  nuove di quella installata (dalla 1.2.2).

## 2026-10-14 — Risoluzione adattiva e viewer (v1.2.2, ADR 0041)
- Sfondo nero durante la sessione: misurato (x264, 1080p) e scartato, risparmio solo su keyframe e spostamento
  finestre; su Windows richiederebbe di cambiare lo sfondo dell'utente.
- Risoluzione ridotta con poca banda: 75 % sotto 0,8 Mbit/s, 50 % sotto 0,45, isteresi e intervalli minimi; nuovo
  encoder alla nuova dimensione e `StreamStarted` al viewer. Lo schermo dell'host non cambia mai.
- Viewer: canvas a tutta finestra con proporzioni mantenute, clic sulle bande nere ignorati; statistiche a larghezza
  fissa. checks.yml verde sul ramo; non ancora provato nell'app compilata.

## 2026-10-14 — Ripresa della cattura, tasti, host sveglio (v1.2.3, ADR 0042, 0043)
- Segnalazione: collegandosi a un Mac sulla finestra di accesso, dopo la password l'immagine resta ferma. Dal log
  dell'host: display 198 a 1920×1080 sulla finestra di accesso, display 199 a 5376×3360 dopo l'accesso; lo stream
  ScreenCaptureKit restava legato al display sparito e la sessione non si accorgeva della fine della cattura.
  Correzioni: delegato di stop e controllo del display su macOS, riavvio dello stream dalla sessione (stesso schermo
  o principale), duplicazione Windows che non termina sugli errori transitori, cattura condivisa terminata non
  riusata (con test).
- Tasti tenuti premuti: il viewer scartava le ripetizioni, ma su macOS e Windows i tasti simulati non si ripetono;
  ora inoltrate (scartate sugli host Linux, dove X ripete da sé).
- Mac in blocco schermo durante la sessione: asserzione IOKit contro lo stop del monitor e attività dichiarata a
  ogni input; su Windows `SetThreadExecutionState`.
- Statistiche: larghezza minima invece che fissa (i simboli ↓ ↑ · sono più larghi di 1ch e il testo veniva tagliato
  con i puntini), contorno giallo/rosso con soglie, "Resync per minuto"; puntatore a freccia.
- Toolchain Rust temporanea nella cartella di lavoro: clippy pulito su macOS e sulla cattura Windows; i test non
  linkano in locale (manca parte del toolchain Swift), girano in CI.

## 2026-10-14 — Clic sul Mac dopo il cambio di schermo (v1.2.4)
- 1.2.3 provata: la cattura riparte da sola passando dalla schermata di blocco e rientrando (log: display 205 →
  206 a 1920×1080 → 207 a 5376×3360).
- Però i clic non funzionavano più (il puntatore sì): enigo su macOS calcola la posizione dei clic con il display
  principale trovato alla creazione (`CGDisplay::main()`), sparito dopo lo scambio. Ora l'input ricrea enigo quando
  cambia l'area (cioè quando la cattura riparte su un altro schermo).
- "Resync per minuto": sempre visibile e ricalcolato con l'orologio reattivo della pagina (prima la cifra cambiava
  solo all'arrivo di un nuovo resync e restava ferma).
- Aperto: Invio remoto sulla schermata di accesso del Mac rifiutato come password errata.

## 2026-10-14 — Nitidezza, modificatori bloccati (v1.2.5)
- "Resync per minuto" fermo a 14: la finestra di controllo usciva da `onMount` prima di avviare l'orologio, quindi
  `now` restava all'apertura e i resync non scadevano mai.
- Blocco dopo uno screenshot (⌘⇧4) sul Mac di chi controlla: ⌘ e ⇧ inoltrati, rilascio preso dal sistema; sul Mac
  controllato restavano premuti. Ora il viewer confronta i flag di ogni evento mouse/tastiera con i tasti premuti e
  rilascia quelli spariti.
- Nitidezza (richiesta utente): massimo 2560×1600, soglie per pixel, limite a 1080p se la codifica è lenta, riduzione
  a media d'area con test. Test locali ora possibili con `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`.

## 2026-10-14 — Audio e HD (v1.3.0, ADR 0044)
- 1.2.5 provata: "Resync per minuto" e ⌘⇧ dopo lo screenshot funzionano (il Mac di chi controlla era rimasto alla 1.2.4).
- Pulsante HD: `HighResolution { enabled }`, il loop di codifica limita a 1080p di pixel quando è spento (subito,
  senza attendere gli intervalli), mantenuto nei riavvii della cattura.
- Audio: nuovo crate `audio` (cattura ScreenCaptureKit / WASAPI loopback, Opus con il crate `opus`, ricampionamento
  lineare, riproduzione cpal con coda e mascheramento delle perdite), canale `audio` non ordinato senza
  ritrasmissioni, pulsante "Audio". Prova da terminale non possibile (manca il permesso Registrazione schermo al
  terminale); riproduzione avviata correttamente.
- Il job Windows di checks.yml ora fa clippy e test di `audio` e `stream`: ha fatto emergere due avvisi clippy
  nell'agente Windows, corretti.
- Aggiornamenti: controllo all'apertura della sezione e ogni 4 ore.

## 2026-10-14 — Nome utente, blocco input, stato (v1.4.0, ADR 0045)
- Richieste utente: pallino verde/giallo/rosso accanto al tipo di connessione, blocco di mouse e tastiera del
  computer controllato, nome utente per entrambi, barra su due righe; "Resync per minuto" di nuovo solo se > 0.
- Nome: in `Proof`/`Accepted` dell'handshake, quindi solo dopo la password verificata; `clean_name` su entrambi i lati.
- Blocco: `input::block` (macOS event tap con marcatore sugli eventi di enigo, Windows hook di basso livello),
  Esc ×3 per riprendere il controllo; messaggi `BlockInput`/`InputBlocked`.
- Sessioni dell'host rimaste aperte dopo la chiusura dal viewer (log: niente `Bye`, niente `leave`, WebRTC sempre
  connesso): causa lato viewer da chiarire; aggiunto timeout di 20 s senza messaggi su entrambi i lati.
- CI: un avviso di codice inutilizzato su Linux e la stampa di peer-poc cambiata facevano fallire i test, corretti.

## 2026-10-14 — Avanzamento file e chat flottante (v1.5.0)
- Domanda utente sul limite del copia/incolla: 500 MB e 10.000 file per copia, testo fino a 1 MB.
- Barra di avanzamento con velocità: contatore sui chunk inviati e scritti, al massimo ogni 250 ms, velocità
  smussata; "finished" quando il contatore viene rilasciato (anche su errore o annullamento).
- Chat: lampeggio del pulsante con messaggi non letti; sul computer controllato la finestra principale non viene più
  portata davanti, si apre invece una finestrella di chat (capability estesa a `chat-*`); i messaggi scritti in una
  finestra compaiono nelle altre (`HostChatSent`).
- Regola dall'utente: release minori (x.y.Z) per le modifiche piccole.

## 2026-10-14 — Volume e qualità (v1.5.1)
- Audio provato dall'utente: funziona. Aggiunto lo slider del volume al posto di "Schermo intero" (si usa il
  pulsante della finestra).
- Più qualità senza banda in più a schermo fermo: bitrate massimo proporzionale ai pixel oltre il 1080p (fino a
  6 Mbit/s), keyframe periodico ogni 60 s invece di 10 s.

## 2026-10-14 — Interrompere i trasferimenti (v1.5.2)
- 1.5.1 includeva anche "Modifica/Salva" per il nome (richiesta arrivata mentre la release era in coda).
- ✕ accanto alla barra di avanzamento: `ClipboardShare::cancel_transfer` ferma l'invio (task annullato) e/o
  fa scartare al lettore il trasferimento in arrivo (`Notify`); il partner riceve `TransferCancelled` e fa lo stesso.

## 2026-10-15 — Invio sulla schermata di accesso
- L'utente conferma che l'Invio da remoto sulla schermata di accesso del Mac ora funziona: domanda chiusa in STATO.

## 2026-10-15 — Connessioni che cadevano (v1.5.3)
- Analizzato il log del Mac controllato: le sessioni venivano chiuse dal server quando il WebSocket del partner
  cadeva o si ricollegava (Cloudflare lo ricicla ogni 30–90 min), anche con la connessione diretta attiva.
- Il client ora ignora queste chiusure per le sessioni avviate (solo "peer left" conta); messaggio corretto
  sull'host quando il partner esce (ADR 0046). ADR 0045 rivisto.
- Log del viewer: conferma la sequenza (WebSocket perso → ricollegato → host chiude → viewer chiude per silenzio).
  Le due sessioni parallele erano un secondo "Connetti" con la prima finestra ancora aperta: ora "Connetti" porta in primo piano la finestra già aperta (richiesta dell'utente).

## 2026-10-15 — Immagini negli appunti (v1.5.4)
- Lo screenshot con ⌘⌃⇧4 non arrivava: gli appunti condivisi gestivano solo testo e file. Ora le immagini
  passano come PNG con `Manifest.image` e arrivano negli appunti come immagine (ADR 0047). Test di codifica e
  trasferimento; prova reale da fare tra due Mac e tra Mac e Windows.
- Release 1.5.4: il job Linux è fallito caricando `latest.json` ("Not Found" sull'aggiornamento dell'asset, corsa
  tra i job) e il file è rimasto con le sole voci Mac. Rilanciato il job Linux, poi aggiunte a mano le tre voci
  Windows con le firme `.sig` pubblicate (stessa struttura della 1.5.3): 11 piattaforme, URL tutti 200.

## 2026-10-15 — Computer senza monitor (v1.6.0)
- L'utente segnala sessioni chiuse dopo 2 s su un PC Windows e un Mac senza monitor (`no displays available`).
- Host: sessione aperta senza schermo (`ScreenUnavailable`, nuovo tentativo ogni 5 s); provato con un host senza
  permesso di registrazione: la sessione resta aperta e il viewer riceve il messaggio.
- macOS: schermo virtuale `CGVirtualDisplay` 1920×1080 quando non c'è nessuno schermo attivo; provato su questo Mac
  (creato e tolto). Da provare su un Mac davvero senza monitor. ADR 0048.

## 2026-10-15 — Monitor virtuale su Windows (v1.7.0)
- Richiesta: un .exe che crei un monitor virtuale 1080p su Windows, installabile e aggiornabile anche dall'app.
- Scelto Virtual Display Driver (MIT, firmato SignPath Foundation), scaricato e verificato in compilazione;
  `KronosVirtualMonitor.exe` lo installa con SetupAPI e lo imposta a 1920×1080@60 (ADR 0049). Scheda nella sezione
  Aggiornamenti; il programma viaggia nell'installer dell'app e da solo nella release.
- Controllato solo con clippy per Windows da qui; da provare su un PC Windows.

## 2026-10-15 — Online/Offline dei dispositivi (v1.7.1)
- Nuovo messaggio `presence` nel protocollo, nel Worker e nel server Rust, con limiti contro la scansione degli ID;
  etichetta Online/Offline nella lista dei dispositivi (ADR 0050).
- Worker provato in locale con `wrangler dev` (Node 26 di Homebrew: wrangler non parte con il Node 20 di nvm): un
  dispositivo online, richiesta troppo ravvicinata rifiutata, dispositivo uscito offline, 21 ID rifiutati.
- Worker pubblicato in produzione (account kronosweb.cloud) e verificato con la stessa prova.
- Domanda dell'utente su più di 20 dispositivi: la prima versione chiedeva solo i primi 20 (gli altri sarebbero
  risultati Offline). Ora gruppi di 20 a rotazione, con test.

## 2026-10-15 — Disconnetti che non arrivava (v1.7.2)
- Segnalazione: dopo Disconnetti il computer controllato è rimasto in modalità controllata. Causa trovata nel
  server: un ricollegamento di uno dei due dispositivi cancellava la sessione, quindi il `leave` successivo non
  arrivava più al partner. Ora la sessione resta (Worker e server Rust, test `leave_reaches_the_partner_after_a_reconnect`).
- `Bye` atteso prima di chiudere; OK sul resoconto chiude tutto (`end_session`). ADR 0051.
- Worker pubblicato (versione 7ecee978). Nei primi minuti il Durable Object ha continuato a cancellare le sessioni
  (codice vecchio ancora attivo: le WebSocket in ibernazione non si scollegano a un deploy); dopo, tre prove su tre
  in produzione: sessione, ricollegamento di chi controlla, `leave` → il partner riceve "peer left".

## 2026-10-16 — Condivisioni doppie verso lo stesso partner (v1.7.3)
- Segnalazione con schermata: il Mac controllato (app 1.7.0) mostrava quattro condivisioni attive verso lo stesso
  controllante, tutte in rete locale. Il viewer chiudeva già le sessioni vecchie verso lo stesso partner, l'host no.
- Ora l'host ferma le sessioni precedenti dello stesso partner quando una nuova supera l'autenticazione. Perché
  le vecchie siano rimaste vive (il controllo dei 20 s le avrebbe chiuse se il controllante fosse sparito) resta
  da capire con i log del controllante.

## 2026-10-16 — Cartelle dei dispositivi e Quick Support stabile (v1.8.0)
- Richieste: Quick Support con ID fisso e password per l'accesso non presidiato; cartelle in I miei dispositivi.
- Quick Support ora usa la sua cartella dati (ID fisso, segreto della password personale); l'accesso non presidiato
  vale finché la finestra è aperta (niente avvio automatico né barra). ADR 0052, rivede la 0031.
- Cartelle annidate (`groups.json`), sposta con menu, apri/chiudi salvato; eliminando una cartella il contenuto sale
  di un livello. Test in `store.rs`; interfaccia controllata con dati finti in Chrome headless.

## 2026-10-16 — Trasferimento file e resoconto a tempo (v1.8.1)
- Richieste: il popup di fine sessione si chiude dopo 30 s; pulsante "Trasferimento file" con interfaccia divisa,
  home dell'utente come cartella iniziale.
- Nuovo canale `fs` e modulo `stream::fs` (ADR 0053). Il test con due peer WebRTC reali ha trovato un caso: un
  annullamento arrivato a copia appena finita lasciava il file. Ora chi riceve scarta subito la copia annullata e
  non salva un `D` di una copia non più attesa; il test passa più volte di fila.
- Interfaccia controllata con dati finti in Chrome headless. Da provare tra due computer, anche Windows e Mac (permesso
  di accesso ai file su macOS).

## 2026-10-16 — Modalità Organizza per le cartelle (v1.8.2)
- Segnalazione: con il menu accanto a ogni dispositivo si rischiava di spostarlo per errore. Proposte tre soluzioni
  (Organizza, menu ⋯ con finestra, trascinamento); l'utente ha scelto Organizza.
- Fuori da Organizza la lista non sposta né modifica nulla; in Organizza selezione multipla, "Sposta in…" con
  conferma e Annulla per 10 s. ADR 0054. Solo interfaccia: backend invariato.
- Controllata con dati finti in Chrome headless.

## 2026-10-19 — Campo password invisibile sul Mac con monitor in stop (v1.8.3)
- Segnalazione: collegandosi a questo Mac dal portatile si vedeva lo sfondo ma non il campo della password; serviva
  disconnettersi e ricollegarsi.
- Causa dal log dell'host: a ogni primo collegamento "no monitor: sharing a virtual one". Il monitor in stop non è
  tra i display attivi, quindi l'host creava lo schermo virtuale (ADR 0048) mentre la schermata di blocco compariva
  sul monitor vero. Al secondo collegamento il monitor era acceso.
- Correzione (ADR 0055): monitor in stop riconosciuto e risvegliato, attesa fino a 5 s; schermo virtuale lasciato se
  un monitor vero torna attivo. Provato su questo Mac con `pmset displaysleepnow`: Asleep, poi On in 20 ms.
- Nota: gli strumenti locali in scratchpad erano stati in parte puliti dal sistema (manca `settings.toml` di rustup
  e cmake): servono `RUSTUP_TOOLCHAIN=stable-aarch64-apple-darwin` e cmake da `pip install cmake`
  con `CMAKE_POLICY_VERSION_MINIMUM=3.5`.
- Workflow di release: con la 1.8.2 due compilazioni avevano creato la release nello stesso secondo (i file Windows
  erano finiti in una seconda copia, spostati a mano). Ora un primo job crea la release come bozza, le quattro
  compilazioni caricano lì (`releaseId`) e un job finale la pubblica solo se `latest.json` ha tutte le 11 piattaforme.

## 2026-10-19 — Windows raggiungibile dopo il riavvio; esporta/importa dispositivi (v1.8.4)
- Segnalazione: dopo il riavvio di Windows, fermo sulla schermata di accesso, non ci si collegava. Causa: l'app
  parte solo all'accesso dell'utente; il servizio SYSTEM fa solo cattura e input.
- Il servizio ora avvia, quando nessuno è entrato, una copia dell'app senza finestre come SYSTEM con ID e password
  personale dell'utente; all'accesso lascia il posto all'app dell'utente ma, su richiesta dell'utente, senza
  interrompere la sessione in corso (ADR 0056). Il servizio va aggiornato una volta (UAC) perché esegue una copia
  protetta dell'app.
- Richiesta durante il lavoro: esportare e importare la lista dei dispositivi. File in Download, cifrato con
  password se contiene le password salvate (ADR 0057). Test in `devices_file.rs` e `store.rs`; interfaccia controllata
  con dati finti in Chrome headless.
- Senza macchina virtuale Windows: `agent` compila per Windows da qui (clippy pulito), il resto lo verificano i
  controlli su Windows di GitHub. Prima prova vera sul PC dell'utente.
- Segnalazione durante il lavoro: dopo lo stop del Mac che controllava, la finestra della sessione restava aperta
  con 0 kbit/s in entrambe le direzioni (nemmeno i ping uscivano): il ciclo della sessione era fermo su un'attesa.
  Protezione: invio dei messaggi di controllo con limite di 3 s, chiusura della connessione con limite di 5 s.
- Causa trovata nel log del Mac che controllava: la sessione era in rete locale; con lo stop si è chiuso il
  collegamento TCP di rete locale (serve solo ad avviare la sessione) e `pump_lan_signals` interrompeva il compito
  della sessione senza pulizia. Restava una sessione "fantasma": finestra aperta, Disconnetti senza effetto, nuovi
  collegamenti allo stesso dispositivo rimandati alla finestra morta. Ora la sessione sopravvive alla chiusura del
  collegamento di rete locale, e le sessioni il cui compito non esiste più vengono riconosciute e chiuse.
- Mac prima dell'accesso, prove sul Mac dell'utente (con il suo permesso, programma di prova installato e poi tolto):
  i permessi di schermo valgono per una copia `.app` con identificativo e firma dell'app; sulla schermata di login
  cattura e puntatore funzionano, tastiera e clic no. Funzione non fatta per il Mac (ADR 0056, sezione "E il Mac?").

## 2026-10-19 — Windows con Desktop remoto (v1.8.5)
- Segnalazione: server Windows senza monitor, gestito anche con RDP; collegandosi con Kronos dopo aver chiuso RDP
  "Failed to find DXGI output" e input rifiutato ("The parameter is incorrect"). Causa: la sessione utente era
  scollegata (niente schermo né input); con RDP aperto Kronos vedeva lo schermo della sessione RDP.
- Rileggendo il codice della 1.8.4: in quel caso il servizio avrebbe avviato l'host prima dell'accesso con lo stesso
  ID dell'app nella sessione scollegata. Corretto: l'host parte solo senza utenti in nessuna sessione. Messaggio
  chiaro al viewer quando la sessione è scollegata.

## 2026-10-20 — Trasferimento file bloccava l'app su Windows (v1.8.6)
- Segnalazione: su Windows, aprendo il trasferimento file l'app si bloccava del tutto. Causa: `open_files_window`
  era un comando sincrono che crea una finestra; su Windows Tauri esegue i comandi sincroni sul thread principale,
  che serve anche alla finestra nuova (stallo). Ora il comando è asincrono, come vanno create le finestre.
- Discusso il Microsoft Store: possibile per l'app completa con il suo installer EXE (serve la firma con un
  certificato riconosciuto: SignPath o certificato a pagamento), pubblico privato limitato agli account Microsoft
  personali; Quick Support resta sul download diretto. In attesa della firma.

## 2026-10-20 — Mouse spostato dopo aver staccato un monitor (v1.8.7)
- Segnalazione: controllando da Windows il Mac 160 809 053 e staccando il monitor in uso, video e tastiera
  funzionavano ma i clic finivano più in alto. Dal log del Mac: display 1 sparito, cattura ripresa sul display 7
  (ADR 0042). macOS ridispone gli schermi rimasti dopo il distacco: la posizione letta al riavvio della cattura
  poteva essere quella vecchia e non veniva più aggiornata, quindi l'area del mouse restava spostata.
- Ora l'host ricontrolla ogni 2 s la posizione dello schermo condiviso e, se cambia, aggiorna l'area dell'input.

## 2026-10-20 — Codifica video hardware (v1.9.0)
- Richiesta: testo sfocato rispetto a VNC; dai log, OpenH264 troppo lento a 2560×1440 e risoluzione ridotta a
  1920×1080. L'utente ha chiesto la codifica sul chip video, con attenzione al carico del sistema.
- Primo tentativo con chiamate dirette a VideoToolbox interrotto (filtro di sicurezza automatico); scelto con l'utente
  un FFmpeg minimo come ponte verso i codificatori hardware, con due patch (pacchetto subito, bitrate al volo) e uno
  strato C. Mac senza copie (CVPixelBuffer di ScreenCaptureKit direttamente al codificatore). ADR 0058.
- Misurato su questo Mac con uno strumento firmato come l'app (per il permesso di registrazione schermo): processore
  da 20,3 s a 1,2–1,6 s su 20 s, 22 → 29 fps, latenza 115 → 17 ms; decodifica WebCodecs verificata in Chrome.
- Windows (Media Foundation + swscale) compilato nei workflow su un ramo di prova; prima prova reale sul PC
  dell'utente.
