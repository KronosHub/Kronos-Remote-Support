# 0026 — Build e test nel repository pubblico

> **Superata da ADR 0040.**

**Data:** 2026-10-08 · **Stato:** accettata (scelta utente) · modifica ADR 0014 e 0023

## Contesto
Minuti gratuiti di GitHub Actions del repo privato esauriti (le build macOS valgono 10x): i job non partono più.

## Decisione
- I workflow `release.yml` e `checks.yml` vivono nel repository delle release precedente (pubblico: minuti gratuiti),
  **solo avvio manuale** (`workflow_dispatch`), nessun trigger da pull request esterne.
- Il codice privato viene letto con una **deploy key SSH di sola lettura** (secret `SOURCE_SSH_KEY` del repo pubblico,
  `persist-credentials: false`). Le release si pubblicano nello stesso repo con il `GITHUB_TOKEN` interno.
- Secret della firma updater duplicati nel repo pubblico. `RELEASES_TOKEN` del repo privato non serve più.
- Nel repo privato i workflow restano ma solo con avvio manuale.

Comandi:
```
gh workflow run checks.yml  -R <repository-delle-release> -f ref=main
gh workflow run release.yml -R <repository-delle-release> -f tag=vX.Y.Z   # dopo aver pushato il tag
```

## Conseguenze
- I log delle build sono pubblici (nomi dei file e output del compilatore, non il codice). Nessun segreto nei log.
- Chi ha accesso in scrittura al repo pubblico può leggere il codice privato tramite la deploy key: tenerlo personale.
