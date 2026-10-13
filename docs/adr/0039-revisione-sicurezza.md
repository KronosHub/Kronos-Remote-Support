# 0039 — Revisione di sicurezza prima dell'apertura del codice

**Data:** 2026-10-13 · **Stato:** accettata

## Contesto
Prima di rendere pubblico il codice (per la firma gratuita con SignPath Foundation) è stata fatta una revisione di
sicurezza su tre fronti: dati sensibili, autenticazione/signaling, superficie d'attacco sull'host. Questa ADR
raccoglie le correzioni della 1.2.0 e i rischi residui accettati.

## Decisioni
- **Servizio Windows (SYSTEM).** Il servizio non esegue più l'eseguibile dell'app (scrivibile dall'utente: chi lo
  sostituiva otteneva SYSTEM), ma una sua copia in `C:\Program Files\Kronos Agent\<servizio>\kronos-agent.exe`,
  cartella scrivibile solo dagli amministratori. Il percorso viene da `SHGetKnownFolderPath`, non da `%ProgramFiles%`
  (un processo elevato eredita l'ambiente dell'utente). I servizi installati prima della 1.2.0 si spostano da soli
  sulla copia protetta al primo avvio. Quick Support installa il servizio ad avvio manuale e lo rimuove (con la copia)
  all'uscita.
- **Log del servizio** in `C:\Program Files\Kronos Agent\logs` invece di `%ProgramData%`, dove un utente poteva
  creare prima la cartella o un collegamento (junction) e far scrivere/cancellare file a SYSTEM.
- **L'agente** accetta solo l'app per cui è stato installato (percorso passato dal servizio) o sé stesso, nella stessa
  sessione: dopo un aggiornamento dell'app non serve reinstallare il servizio.
- **Password temporanea:** dopo 3 tentativi errati viene cambiata (niente più blocco globale di 10 minuti che
  chiunque poteva provocare). I tentativi fatti in parallelo verificano tutti la stessa password: se nel frattempo è
  stata cambiata, anche un tentativo giusto non vale.
- **Password personale:** 5 errori in 10 minuti sospendono solo l'accesso non presidiato; anche qui un tentativo
  giusto arrivato dopo il blocco viene scartato.
- **Rete locale:** massimo 8 connessioni dirette aperte, presentazione entro 10 s, righe di al massimo 64 KB.
- **Handshake:** la decodifica esadecimale non va più in panico con caratteri non ASCII.
- **`device.key`** creata con permessi 0600 (e corretti sui file esistenti) su macOS/Linux.
- **Chat:** messaggi tagliati a 4000 caratteri, al massimo 20 ogni 10 s per sessione, la finestra viene portata in
  primo piano al massimo ogni 30 s.
- **Appunti:** con la condivisione spenta i file ricevuti vengono scartati senza scrivere nulla; cartella di
  ricezione privata (0700, `$XDG_RUNTIME_DIR` o cartella nuova e univoca) invece di `/tmp/kronos-clipboard`;
  percorsi validati tutti prima di scrivere; somma delle dimensioni senza overflow. L'host può rifiutare la
  condivisione degli appunti (`StreamConfig::allow_clipboard`, per ora sempre attivo: manca l'interfaccia).
- **Server di signaling (Worker):** limiti per IP su connessioni, messaggi, richieste di connessione e nuovi ID;
  credenziali TURN solo nelle sessioni e con un tetto orario per IP; tabelle in memoria limitate.
- **Pipeline:** azioni GitHub fissate a un commit preciso; permessi minimi.

## Rischi residui accettati
- Un programma malevolo che gira già come l'utente può avviare l'app vera (o iniettarsi in essa) e usare l'agente per
  agire sulle finestre di amministratore e sul desktop protetto. Non si chiude senza togliere la funzione; Microsoft
  non considera il consenso UAC un confine di sicurezza per gli amministratori.
- L'ID dichiarato da chi si collega in rete locale non è verificato (la verifica vera è la password).
- Le credenziali TURN fisse (ExpressTURN) restano leggibili da chi apre una sessione: la soluzione è passare alle
  credenziali temporanee di Cloudflare TURN.
- Su Windows il segreto della password personale è in chiaro nella cartella dati dell'utente (non cifrato con DPAPI).
