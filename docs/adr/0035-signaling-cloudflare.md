# 0035 — Signaling su Cloudflare Workers

**Data:** 2026-10-09 · **Stato:** accettata

## Problema
L'utente non può usare il proprio server Oracle per motivi di sicurezza; serve una soluzione gratuita per collegare PC
su reti diverse.

## Decisione
- Signaling in `deploy/cloudflare`: Worker + Durable Object SQLite sul piano gratuito, con lo stesso protocollo JSON
  di `apps/signaling`. ID stabili per dispositivo salvati in SQLite. L'app cambia solo `SERVER`.
- STUN di Cloudflare. TURN opzionale, consegnato dal Worker a ogni sessione da segreti del Worker: credenziali fisse
  (`TURN_URL`, `TURN_USERNAME`, `TURN_PASSWORD`, es. piano gratuito ExpressTURN) oppure chiave Cloudflare TURN
  (richiede un metodo di pagamento: scartata). Solo URL UDP. Nessuna credenziale nell'app.
- `apps/signaling` resta per i test locali.

## Conseguenze
- ID nuovi per tutti i dispositivi: la password personale (legata all'ID) viene disattivata con un avviso; i "Miei
  dispositivi" vanno riaggiunti. Vanno aggiornati tutti i PC.
- Senza TURN le reti molto chiuse non si collegano; la rete locale funziona comunque.

## Verifica
Servizio in locale (`wrangler dev`) e pubblicato: sessione reale host/viewer a 30 fps, password errata rifiutata, PC
offline segnalato, ID stabile al riavvio.

## Altre modifiche nella stessa versione
- Password temporanea di 8 caratteri con almeno 5 cifre.
- Nuova password temporanea alla fine di ogni connessione.
