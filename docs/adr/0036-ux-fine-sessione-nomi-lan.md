# 0036 — Nomi LAN, popup di fine sessione, barra riordinata

**Data:** 2026-10-09 · **Stato:** accettata

## Richieste utente
- Nei dispositivi LAN mostrare l'hostname e poter assegnare un nome.
- A fine connessione un popup con la durata, su host e guest.
- Sezione "Attività" nascosta, visibile da un pulsante.
- Barra informazioni del viewer più ordinata, ad aree.

## Decisioni
- **Nomi LAN**: l'annuncio mDNS usa l'hostname reale (`gethostname`, senza suffisso dominio). L'app salva un alias
  per ID in `prefs.json` (`set_lan_name`), mostrato al posto dell'hostname; elenco con alias/hostname/ID e pulsante
  "Rinomina" (finestra interna, non `window.prompt`).
- **Fine sessione**: `ViewClosed`/`HostingEnded` portano la durata (dalla connessione); l'app mostra un popup nella
  finestra principale (riportata in primo piano) con "Sessione con \<peer\> terminata", durata in evidenza, ruolo e ora.
- **Attività**: sezione log nascosta dietro il pulsante "Attività" nell'intestazione.
- **Barra viewer**: riquadri delimitati con etichetta (Video, Rete, Risorse, Qualità ridotta, Risinc.).
- **Fix**: alla chiusura della sessione dal server l'app ora termina esplicitamente vista e condivisione (prima
  l'immagine restava congelata senza popup).
