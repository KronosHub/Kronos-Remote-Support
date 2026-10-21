# 0059 — Windows usato solo con Desktop remoto

**Data:** 2026-10-21 · **Stato:** accettata (v1.9.1)

## Problema
Segnalazione dell'utente su una VM Windows gestita solo con Desktop remoto (RDP): dopo un riavvio Kronos non era
raggiungibile finché non si entrava in RDP, anche con "Avvia all'accesso"; chiudendo RDP senza uscire la macchina
non si vedeva.
- "Avvia all'accesso" parte solo quando qualcuno entra in Windows: prima dell'accesso serve l'host della 1.8.4
  (ADR 0056), che però copiava ID e password personale solo dall'utente **alla console**. Su una VM usata solo via
  RDP l'utente non è mai alla console: niente copia, niente host dopo il riavvio.
- L'host prima dell'accesso si ritirava solo quando qualcuno entrava nella **sua** sessione (console); entrando in
  RDP restava acceso accanto all'app dell'utente, con lo stesso ID.
- Una sessione RDP chiusa senza uscire resta "scollegata": niente schermo né input (1.8.5), l'app vi gira ma non
  c'è nulla da mostrare.

## Decisione
- I file dell'host prima dell'accesso si copiano dall'utente alla console o, se lì non c'è nessuno, da un utente
  collegato in un'altra sessione (RDP) che ha usato l'app. L'host si ritira quando un utente entra in qualunque
  sessione. `SERVICE_REVISION` 3.
- **Opzione "Computer usato solo da remoto (senza monitor)"** (`remote_only` in `prefs.json`, copiato per il
  servizio): chiesta quando si attiva l'accesso non presidiato su Windows, modificabile dopo. Con l'opzione attiva,
  il servizio sposta sulla console la sessione RDP scollegata di un utente che usa l'app, se alla console non c'è
  nessuno, come `tscon <id> /dest:console`: avvia `app --to-console <id>` come SYSTEM nella sessione della console
  (`WTSConnectSessionW`). La sessione torna ad avere schermo e input e Kronos la condivide; un nuovo collegamento
  RDP la riprende come sempre. Al più un tentativo ogni 30 s.
- Domanda esplicita invece di un automatismo: su un PC con un monitor la sessione comparirebbe **sbloccata**
  davanti a chiunque sia lì; per questo l'opzione è spenta di default.

## Conseguenze
- Richiede il servizio (controllo finestre di amministratore) attivo.
- Da provare su una VM vera: riavvio senza accesso, accesso RDP, chiusura di RDP senza uscire.
