# ADR 0049 — Monitor virtuale su Windows (Virtual Display Driver)

**Data:** 2026-10-15
**Stato:** accettato

## Contesto
Un PC Windows senza monitor non ha uno schermo da catturare: dalla 1.6.0 la sessione resta aperta ma senza
immagine (ADR 0048). Su Windows un monitor virtuale richiede un driver video (Indirect Display Driver) firmato;
scriverne e firmarne uno nostro non è possibile finché non abbiamo la firma del codice.

## Decisione
- Si usa **Virtual Display Driver** (github.com/VirtualDrivers/Virtual-Display-Driver, licenza MIT, driver
  UMDF firmato da SignPath Foundation con certificato GlobalSign), versione 25.7.23, pacchetto x64 "Driver Only".
  Il driver non è nel repository: `tools/virtual-monitor/fetch-driver.sh` lo scarica dalla release ufficiale e
  ne verifica lo SHA-256 prima della compilazione. La licenza è in `tools/virtual-monitor/`.
- **KronosVirtualMonitor.exe** (`tools/virtual-monitor`) incorpora il pacchetto e:
  - `install`: scrive `C:\VirtualDisplayDriver\vdd_settings.xml` (un monitor, solo 1920×1080 a 60 Hz), toglie
    eventuali dispositivi `Root\MttVDD`, crea il dispositivo (SetupAPI, `DIF_REGISTERDEVICE`) e installa il driver
    (`UpdateDriverForPlugAndPlayDevicesW`); Windows chiede conferma per l'editore SignPath Foundation (non lo
    aggiungiamo agli editori attendibili). Scrive la versione in `kronos-virtual-monitor.txt` nella stessa
    cartella;
  - `uninstall`: rimuove i dispositivi, il pacchetto dal driver store (`SetupUninstallOEMInfW`) e la cartella;
  - `status`: stato e versioni, senza permessi di amministratore;
  - senza argomenti (doppio clic): menu in italiano.
  Non ha il manifest "requireAdministrator": per installare e rimuovere si rilancia da solo come amministratore
  (UAC), così `status` non chiede nulla. Esito nel codice di uscita e, con `--result <file>`, come testo.
- **App**: l'installer Windows include il programma (`tauri.windows.conf.json`, risorsa
  `virtual-monitor/KronosVirtualMonitor.exe`); la sezione Aggiornamenti mostra la scheda "Monitor virtuale"
  (Installa, Aggiorna se la versione portata dall'app è diversa da quella installata, Rimuovi). Così un
  aggiornamento dell'app porta anche il programma nuovo. Non in Quick Support (nessun installer).
- **Release**: il programma è compilato prima dell'app e caricato anche da solo (`KronosVirtualMonitor.exe`);
  i controlli Windows ora compilano il programma e il codice Windows dell'app.

## Conseguenze
- Dipendiamo da un driver di terzi: aggiornarlo vuol dire cambiare versione e SHA-256 in `fetch-driver.sh`.
- Il programma, come l'app, non è firmato: SmartScreen avvisa finché non arriva SignPath.
- Non provato qui (nessun PC Windows): il primo test è sul campo.
