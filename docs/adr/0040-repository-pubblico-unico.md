# ADR 0040 — Repository pubblico unico

Stato: accettata (2026-10-13). Supera ADR 0026.

## Contesto
Fino alla 1.2.0 il codice stava in un repository privato e le build/release in un repository pubblico
separato, che scaricava il sorgente con una chiave SSH (ADR 0026). Con l'apertura del codice (ADR 0039)
e la richiesta di firma gratuita a SignPath Foundation serve un repository pubblico con il sorgente.
La cronologia del repository privato contiene dati che non devono diventare pubblici.

## Decisione
- Un solo repository pubblico, `KronosHub/Kronos-Remote-Support`: sorgente, CI e release.
- Creato da un solo commit (nessuna cronologia precedente); la cronologia resta nel repository privato,
  da archiviare.
- Workflow in `.github/workflows/`: `checks.yml` (push su `main`, pull request, avvio manuale) e
  `release.yml` (avvio manuale con il tag). Pubblicano con il `GITHUB_TOKEN` del repository: non servono
  più `SOURCE_SSH_KEY` né `RELEASES_TOKEN`.
- Aggiornamenti in-app (ADR 0023): dalla 1.2.1 l'updater interroga prima
  `https://github.com/KronosHub/Kronos-Remote-Support/releases/latest/download/latest.json` e, in
  alternativa, il vecchio repository delle release.
- Passaggio: la 1.2.1 esce sul nuovo repository e il suo `latest.json` viene copiato anche come release
  del vecchio repository, così le installazioni fino alla 1.2.0 la ricevono. Quando non restano
  installazioni precedenti alla 1.2.1, il vecchio repository si archivia.

## Conseguenze
- Le chiavi di firma (updater Tauri, identità macOS) vanno caricate come secret del nuovo repository.
- Il vecchio repository delle release va tenuto finché esistono installazioni fino alla 1.2.0.
