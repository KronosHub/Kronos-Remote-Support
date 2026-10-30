# Architettura

## Obiettivo MVP
Controllare da remoto un desktop Windows/macOS/Linux da un altro desktop Windows/macOS/Linux:
vedere lo schermo, muovere mouse/tastiera, attraversare NAT, connessione cifrata con ID + password.

## Componenti

```
 ┌───────────────────────┐        WebSocket (TLS)        ┌───────────────────────┐
 │   App (Tauri)         │◄──────────────┐ ┌────────────►│   App (Tauri)         │
 │   ruolo: VIEWER       │               │ │             │   ruolo: HOST         │
 │  - UI connessione     │          ┌────┴─┴─────┐       │  - cattura schermo    │
 │  - decodifica video   │          │ Signaling  │       │  - encoding video     │
 │  - invio input        │          │ server     │       │  - iniezione input    │
 └──────────┬────────────┘          │ (Rust/axum)│       └──────────┬────────────┘
            │                       └────────────┘                  │
            │        WebRTC (P2P diretto via ICE/STUN)              │
            └──────────────────────────────────────────────────────►┘
            │        oppure relay se P2P impossibile                │
            └──────────────►  TURN server (coturn)  ◄───────────────┘
```

Un'unica app contiene entrambi i ruoli (come TeamViewer): mostra il proprio ID/password e permette di connettersi ad altri.

### 1. App desktop (Tauri + core Rust)
- **UI**: Tauri 2 + SvelteKit (ADR 0006, 0013); viewer con canvas + WebCodecs.
- **Core Rust** (crate separati, testabili senza UI):
  - `capture`: cattura schermo per piattaforma
    - Windows: DXGI Desktop Duplication (implementato, ADR 0019)
    - macOS: ScreenCaptureKit (permesso Screen Recording)
    - Linux: X11 GetImage (implementato); Wayland via xdg-desktop-portal + PipeWire dopo
  - `codec`: encoding H.264 (viewer: decodifica WebCodecs nella UI, ADR 0015) (openh264 software, poi hardware: Media Foundation/NVENC, VideoToolbox, VAAPI); alternativa VP8/VP9
  - `input`: iniezione mouse/tastiera (SendInput / CGEvent / XTest-uinput); macOS richiede permesso Accessibility
  - `transport`: WebRTC (video track + data channel per input, clipboard, file, controllo)
  - `protocol`: messaggi del data channel (serializzazione binaria, es. bincode/protobuf)
  - `auth`: ID, password monouso, verifica PAKE

### 2. Signaling server (Rust, axum + WebSocket)
- Registrazione dei peer con ID numerico (es. 9 cifre).
- Inoltro offer/answer SDP e candidati ICE tra peer.
- Fornisce credenziali TURN temporanee (TURN REST API, HMAC).
- Non vede mai media né input.
- Hosting: in origine VM Oracle Cloud (ADR 0008), dietro Caddy con TLS su `<ip>.sslip.io` (ADR 0010); dalla v0.12.0
  Cloudflare Workers + Durable Object (`deploy/cloudflare`, ADR 0035). Il server Rust `apps/signaling` è stato tolto nella 2.0.2: i test end-to-end usano il Worker in locale (`wrangler dev`).
- Protocollo: vedi ADR 0010.

### 3. TURN/STUN (coturn)
- Self-hosted, usato solo quando P2P diretto fallisce.

## Flusso di connessione
1. Host avvia, si registra al signaling → riceve/conserva ID, genera password monouso.
2. Viewer inserisce ID → signaling inoltra richiesta all'host.
3. Scambio SDP + candidati ICE via signaling → connessione WebRTC (DTLS).
4. Sul data channel `control`: SPAKE2 con la password + conferma legata ai fingerprint DTLS (ADR 0012).
5. Password corretta: popup sull'host, che accetta o rifiuta (timeout 30 s).
6. Se accettata: l'host cattura e codifica lo schermo e lo invia sulla traccia video WebRTC; il viewer lo
   ricompone e lo decodifica con WebCodecs (ADR 0015). Canale `control`: ping, richieste keyframe, fine sessione.
7. Input dal viewer all'host sul canale `control`, replicato con tasti fisici (ADR 0018).

## Sicurezza
- Trasporto: DTLS-SRTP (WebRTC) end-to-end.
- **Rischio**: il signaling server potrebbe fare MITM sostituendo le fingerprint DTLS negli SDP.
  **Mitigazione**: PAKE (es. SPAKE2) con la password, legando le fingerprint DTLS di entrambi i lati
  nella trascrizione → un server malevolo non può impersonare l'host senza conoscere la password.
- Conferma esplicita dell'host per ogni connessione (implementata).
- Rate limit: 10 `connect`/min per connessione (signaling), 5 password errate/10 min (host). Implementati.
- Da fare: indicatore "sessione attiva" + pulsante per terminare durante il controllo remoto.

## Struttura repository
Esistono già: `crates/{auth,capture,codec,protocol,signaling-proto,stream,transport}`, `apps/{desktop,signaling}`, `tools/{capture-poc,peer-poc}`, `deploy/`, `scripts/`. Il resto è previsto.
```
Desktop_remoto/
├── Cargo.toml              # workspace
├── crates/
│   ├── capture/
│   ├── codec/
│   ├── auth/               # password + SPAKE2 + conferma (ADR 0012)
│   ├── protocol/           # messaggi del canale di controllo (ping, keyframe, bye)
│   ├── stream/             # pipeline host (cattura→H.264→traccia) e logica sessione host/viewer
│   ├── signaling-proto/    # messaggi JSON client<->signaling (ADR 0010)
│   ├── transport/          # client signaling + sessione WebRTC (unico crate che usa webrtc-rs)
│   └── input/              # iniezione mouse/tastiera, tasti fisici (ADR 0018)
├── apps/
│   ├── desktop/            # Tauri 2 + SvelteKit: GUI, servizio in background, tray (ADR 0013)
│   └── signaling/          # server
├── tools/
│   ├── capture-poc/        # PoC Fase 1: cattura + encoding su file
│   └── peer-poc/           # PoC Fase 2: host/connect + ping su data channel
├── scripts/
│   └── e2e-local.sh        # test end-to-end locale (server + host + viewer)
├── deploy/                 # signaling + Caddy su VM Linux, systemd (ADR 0011); cloudflare/: Worker (ADR 0035)
├── .github/workflows/      # ci.yml, release.yml (ADR 0014)
└── docs/
```
