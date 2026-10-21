# 0023 — Aggiornamenti dall'app (Tauri updater) e repository pubblico delle release

**Data:** 2026-10-08 · **Stato:** accettata (scelte utente)

## Requisito
Aggiornare l'app con un pulsante, senza scaricare a mano gli installer.

## Decisione
- `tauri-plugin-updater` 2: comandi `check_update` / `install_update` (download, verifica firma, installazione,
  riavvio) e `app_version`; avanzamento via evento `update_progress`. Controllo silenzioso 4 s dopo l'avvio;
  pulsante "Aggiornamenti" / "Aggiorna a X" nell'intestazione, finestra con note e barra di avanzamento.
- **Firma degli aggiornamenti** (minisign di Tauri, distinta dalla firma del codice macOS): chiave pubblica in
  `tauri.conf.json`, privata + password nei secret `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` del repo privato e in
  backup locale `~/.tauri/desktop-remoto-updater.key*` (0600, fuori dal repo). Senza la chiave privata non si
  possono pubblicare aggiornamenti accettati dalle app installate: **non perderla**.
- **Distribuzione**: il repo del codice resta privato; gli installer vengono pubblicati, come release **non bozza**,
  su un repository pubblico delle release separato (scelta utente). Endpoint dell'updater:
  `https://github.com/<repository-delle-release>/releases/latest/download/latest.json`.
  Oggi codice e release stanno in `KronosHub/Kronos-Remote-Support` (ADR 0040).
- `release.yml` (tauri-action) pubblica lì con il secret `RELEASES_TOKEN` (token fine-grained limitato a quel repo,
  permesso Contents read/write). Windows: aggiornamento tramite installer NSIS in modalità `passive`.
- Build locali: servono le variabili `TAURI_SIGNING_PRIVATE_KEY` e `..._PASSWORD` (dai file in `~/.tauri`).

## Conseguenze
- Chiunque può scaricare gli installer dal repo pubblico (e quindi usare il server di signaling).
- Linux: si aggiorna solo l'AppImage; `.deb`/`.rpm` vanno reinstallati a mano.
- macOS con firma ad-hoc: dopo ogni aggiornamento possono servire di nuovo i permessi (vedi proposta certificato).
- Le versioni ≤ 0.5.x non hanno l'updater: la 0.6.0 va installata a mano una volta.

## Aggiornamento 2.0.1 (2026-10-21): MSI aggiornabile da remoto
- Problema: su un PC installato con l'MSI (per tutti gli utenti) l'aggiornamento chiudeva l'app e solo dopo Windows
  chiedeva il permesso di amministratore (UAC) per `msiexec`: la sessione remota era già caduta e la richiesta
  restava in attesa di qualcuno davanti al PC. Con l'installer NSIS (per utente) l'UAC non serve.
- Ora, se il pacchetto scaricato è un MSI, l'app lo scarica (firma verificata dall'updater), lo scrive nella cartella
  temporanea e avvia `msiexec /i … /passive /promptrestart AUTOLAUNCHAPP=True LAUNCHAPPARGS=…` con il verbo
  `runas` (`agent::windows::spawn_elevated`) **prima** di chiudersi: chi controlla il PC vede la richiesta UAC
  (con il controllo finestre di amministratore attivo, ADR 0033) e la approva; poi l'app si chiude e l'MSI la
  riavvia a installazione finita. Se l'UAC viene rifiutato l'app resta aperta e mostra l'errore.
- La prima volta serve ancora qualcuno al PC: la versione installata è quella vecchia, con il flusso vecchio.
