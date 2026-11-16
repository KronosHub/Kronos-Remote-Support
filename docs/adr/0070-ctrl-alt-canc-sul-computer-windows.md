# 0070 — Ctrl+Alt+Canc sul computer Windows controllato

**Data:** 2026-11-16 · **Stato:** accettata (da verificare sulla VM Windows)

## Contesto
Chi controlla un PC Windows deve poter inviare Ctrl+Alt+Canc (schermata di sicurezza, accesso dalla schermata di
blocco). Windows non accetta la sequenza come tasti iniettati: va chiesta al sistema con `SendSAS` (`sas.dll`), che
risponde solo a un processo LocalSystem (o con uiAccess) e solo se la policy `SoftwareSASGeneration`
(`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System`) consente i servizi (bit 1).

## Decisione
- Protocollo: nuovo `InputEvent::SecureAttention` (`{"k":"secure_attention"}`) dentro `ControlMsg::Input`, così
  passa dallo stesso percorso dell'input fino all'agente SYSTEM (ADR 0033) senza toccare la pipe. Gli host vecchi
  scartano il messaggio. Risposta `ControlMsg::SecureAttentionFailed { message }` quando non si può.
- `crates/input/src/sas.rs` (solo Windows): legge la policy, se manca il bit 1 lo accende, chiama
  `SendSAS(FALSE)` caricando `sas.dll` solo da System32, poi rimette la policy com'era (o la cancella se non c'era).
- Host (`stream::session`): l'evento va all'iniettore solo se è l'agente; con l'iniettore locale (app come utente,
  senza servizio) risponde subito `SecureAttentionFailed` con la spiegazione (serve il controllo UAC o l'accesso non
  presidiato). Il viewer la mostra come avviso (`ViewNotice`).
- Viewer: pulsante **Ctrl+Alt+Canc** nella barra e nel pannello del pallino, solo con partner Windows
  (`Displays.os`), attivo solo con il controllo acceso.

## Alternative scartate
- Iniettare Ctrl, Alt e Canc come tasti: Windows li ignora per la SAS.
- Chiamare `SendSAS` dal servizio in sessione 0: servirebbe un canale nuovo app → servizio; l'agente è già SYSTEM
  nella sessione giusta e riceve già l'input.
- Impostare la policy in modo permanente all'installazione del servizio: cambia la sicurezza del PC anche quando
  Kronos non è in uso.

## Da verificare
Sulla VM: SAS dal desktop, dalla schermata di blocco e con l'UAC aperto; che la policy torni come prima. Se `SendSAS`
rifiutasse un processo SYSTEM non servizio, spostare la chiamata nel servizio.
