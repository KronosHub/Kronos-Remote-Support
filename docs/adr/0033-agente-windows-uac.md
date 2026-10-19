# 0033 — Agente Windows per le finestre di amministratore (UAC) e la schermata di blocco

**Data:** 2026-10-09 · **Stato:** accettata; dalla 1.8.4 il servizio tiene raggiungibile il PC anche prima dell'accesso (ADR 0056)

## Problema
Su Windows i popup di amministratore (UAC) e la schermata di blocco vivono su un desktop separato e sicuro
("Winlogon"), che un processo avviato dall'utente non può né vedere né comandare. Finché l'app gira come utente, chi
assiste non può inserire password negli UAC né agire sulla schermata di blocco.

## Decisione
Un **servizio Windows** che gira come SYSTEM avvia un processo **agente** nella sessione dell'utente alla console.
L'agente segue il desktop che riceve l'input (normale o sicuro), lo cattura e vi inietta mouse/tastiera. L'app
(sempre come utente) fa tutto il resto — signaling, WebRTC, codifica, approvazioni — e parla con l'agente su una
**named pipe** locale. Quando l'agente non c'è, l'app cattura e inietta da sé come prima (nessuna regressione).

### Componenti
- `crates/windesk`: `follow_input_desktop()` (sposta il thread sul desktop di input, `SetThreadDesktop`), usato da
  input e cattura Windows; altrove no-op.
- `crates/agent`: protocollo pipe (lunghezza+tipo: `J` JSON, `F` frame BGRA), lato agente (`serve`, backend `System`
  con `capture::shared` per condividere l'unica duplicazione DXGI tra più sessioni) e lato app (`AgentCapturer`
  implementa `Capturer`, `AgentInput`); su Windows: creazione pipe con DACL protetta (solo SYSTEM e utenti interattivi),
  verifica reciproca (l'agente serve solo lo stesso eseguibile nella stessa sessione; l'app accetta solo una pipe
  creata da un processo privilegiato — proprietario SYSTEM o Administrators), servizio (`windows-service`), avvio
  dell'agente con `CreateProcessAsUserW` sul token SYSTEM nella sessione console attiva, elevazione (`ShellExecuteExW`
  verbo `runas`).
- App: `StreamConfig.agent_product` ("remote"/"quick"); cattura e input passano dall'agente se è in esecuzione.
  Argomenti `--agent` / `--service` / `--install-service` / `--uninstall-service`. Comandi UI `uac_support`,
  `set_uac_support` (app completa: installa/rimuove il servizio con un UAC), `relaunch_elevated` (Quick).
- **App completa**: interruttore "Controllo finestre di amministratore (UAC)" che installa/rimuove il servizio (un UAC
  una volta). **Quick Support**: se avviato come amministratore installa un servizio temporaneo (rimosso all'uscita);
  altrimenti propone "Riavvia come amministratore" oppure si continua con i permessi utente.

### Sicurezza
Pipe per prodotto e sessione (`\\.\pipe\kronos-agent-<prodotto>-<sessione>`), DACL protetta, prima istanza. L'app
verifica che la pipe sia di un processo privilegiato; l'agente verifica che il client sia lo stesso eseguibile nella
stessa sessione. L'agente gira come SYSTEM solo per cattura/input: nessun segreto o rete ci passa.

## Verifica (VM Parallels Windows 11 ARM64, autorizzata dall'utente)
Strumento `tools/agent-poc`. Servizio installato → agente avviato nella sessione console (1), non in sessione 0.
Cattura via agente: schermo e **popup UAC reale** catturati. Iniezione via agente: puntatore portato a 0,25/0,5 →
posizione corretta sullo schermo fisico (DPI gestito dai bounds dell'agente). Verifica reciproca della pipe: il
controllo del proprietario ha scoperto che Windows assegna Administrators come proprietario (non SYSTEM), da cui
l'accettazione di entrambi. Desktop sicuro oscurato e schermata di blocco: non ancora verificati live (questa VM mostra
l'UAC sul desktop interattivo); il meccanismo di inseguimento del desktop è però in funzione.

## Limiti / seguito
- Build dell'app solo in CI su Windows (il cross-build da mac non compila le risorse Tauri).
- Disinstallando l'app completa con un installer non elevato il servizio può restare: va tolto prima con "Disattiva"
  (seguito: hook di disinstallazione elevato).
- Firma del codice (ADR aperta): un servizio SYSTEM non firmato è ancora più sospetto agli antivirus.
