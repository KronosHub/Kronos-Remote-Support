# 0074 — Statistiche grafiche nella barra e nel pannello del pallino

**Data:** 2026-11-16 · **Stato:** accettata

## Decisione
Scelte con l'utente dai mockup (proposta B rivista per la barra, P1 con l'arco di P3 per il pannello):
- **Latenza**: 4 tacche di segnale (`signalBars`: sotto 80 ms 4, 150 3, 300 2, oltre 1), gialle e rosse con le
  soglie di prima.
- **Fotogrammi**: barra (nel pannello un arco) piena a `fpsScale`, cioè 60 con codifica hardware e 30 con quella
  software (il massimo che il partner manda, ADR 0072). Il colore scorre con il valore (`fpsColor`, verde, giallo,
  arancio, rosso sotto un quarto della scala). Sotto, la risoluzione con una pastiglia **GPU** (blu) o **CPU**
  (viola): non sono un errore, solo il modo di comprimere.
- **Banda** ↓/↑ e **CPU** Tu/Partner su due righe in colonna, con barre allineate (banda piena a 6 Mbit/s, il
  massimo del bitrate).
- Numeri con larghezza minima fissa: cambiando ogni secondo non devono far andare a capo la barra.
- Pannello del pallino: riquadri su due colonne, risorse su una riga intera, resync sotto la latenza solo se
  maggiore di 0; azioni in una griglia a tre colonne con l'icona sopra il nome.

## Sfarfallio della barra
La barra "si duplicava" per un attimo ogni secondo. Causa: due istanze dell'app sullo stesso Mac si rimandavano
la stessa immagine degli appunti all'infinito, e la riga del trasferimento compariva e spariva. Ora gli appunti
ricordano l'hash dell'ultima immagine (`image_hash`) e non la rimandano né la riscrivono se è la stessa; la riga del
trasferimento compare solo se dura più di 600 ms (`TRANSFER_ROW_DELAY_MS`). Il vetro della barra
(`backdrop-filter`) resta.
