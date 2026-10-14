# 0042 — Ripresa automatica della cattura

**Data:** 2026-10-14 · **Stato:** accettata

## Problema
Collegandosi a un Mac fermo sulla finestra di accesso, dopo aver inserito la password l'immagine restava ferma e
bisognava disconnettersi e riconnettersi; mouse e tastiera continuavano a funzionare "alla cieca". Dal log
dell'host: sulla finestra di accesso lo schermo era il display 198 a 1920×1080, dopo l'accesso il display 199 a
5376×3360. macOS sostituisce il display e lo stream ScreenCaptureKit legato a quello vecchio smette di mandare
frame. Cause nel codice:
- macOS: nessun delegato per gli stop di ScreenCaptureKit, nessun controllo che il display esista ancora (uno
  stream muto sembrava uno schermo fermo);
- quando la cattura si fermava, il thread di codifica terminava ma la sessione non se ne accorgeva;
- Windows, stesso rischio allo sblocco: la duplicazione DXGI trattava come fatale ogni errore diverso da
  "accesso perso", dopo "accesso perso" la ricreava chiedendo anche formati non BGRA, e se la risoluzione
  cambiava scartava i frame per sempre;
- una cattura condivisa (`capture::shared`) appena terminata poteva essere riusata da una nuova sessione.

## Decisione
- **Sessione host**: `StreamHandle::changed()` segnala anche la fine dello stream (`StreamChange::Ended`); la
  sessione riavvia lo stream sullo stesso schermo o, se non c'è più, su quello principale; riprova ogni 2 s se
  non riesce, aggiorna l'area dell'input, manda `StreamStarted` (il viewer riparte con la nuova dimensione) e la
  nuova lista degli schermi.
- **macOS**: lo stream ha un delegato (`did_stop_with_error`) e, dopo 2 s senza frame, la cattura verifica che
  il display esista ancora; in entrambi i casi termina con un errore e la sessione la riavvia.
- **Windows**: ogni errore della duplicazione (salvo il timeout) la chiude e la riapre al giro dopo; la
  ricreazione chiede solo BGRA; se la risoluzione cambia la cattura termina con un errore.
- **Cattura condivisa**: quando il thread termina segna `ended` (sotto il lock degli iscritti); una nuova
  sessione non si aggancia a una cattura terminata e chi si iscrive in quel momento riceve subito l'errore.

## Conseguenze
- Qualsiasi interruzione della cattura diventa una pausa di un paio di secondi, non un'immagine ferma.
- Con schermo fermo, sul Mac la lista dei display viene interrogata ogni 2 s (costo trascurabile).
- Da verificare sul campo: Mac sulla finestra di accesso e sulla schermata di blocco; Windows con e senza agente.
