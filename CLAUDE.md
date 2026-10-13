# Desktop_remoto

App desktop remoto multipiattaforma stile TeamViewer. Progetto personale/studio.

## Riprendere il lavoro
Leggere in ordine:
1. `docs/STATO.md` — dove siamo, prossimo passo
2. `docs/ARCHITETTURA.md` — componenti e flussi
3. `docs/ROADMAP.md` — fasi e checklist
4. `docs/adr/` — decisioni prese (Architecture Decision Records)
5. `docs/DIARIO.md` — log sessioni

## Regole di tracciamento
- A fine sessione: aggiornare `docs/STATO.md` e aggiungere voce in `docs/DIARIO.md`.
- Ogni decisione tecnica non banale: nuovo file `docs/adr/NNNN-titolo.md`.
- Domande aperte: sezione "Domande aperte" in `docs/STATO.md`.
- Documentazione in italiano; codice, identificatori e commit in inglese.

## Build locale dell'app
Serve la chiave di firma degli aggiornamenti (ADR 0023):
```
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/desktop-remoto-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/.tauri/desktop-remoto-updater.key.password)"
export APPLE_SIGNING_IDENTITY="Kronos Remote Support Signing"   # firma stabile macOS (ADR 0027)
cd apps/desktop && npx tauri build
# Kronos Quick Support (ADR 0031)
npx tauri build --config src-tauri/tauri.quick.conf.json --features quick-access --bundles app
```
Ogni release ha una sezione `## X.Y.Z — AAAA-MM-GG` in cima a `CHANGELOG.md` (novità per l'utente, in italiano):
il workflow la mette nelle note dell'aggiornamento e l'app mostra le versioni più nuove di quella installata.
Proporla all'utente insieme alla release.

Test e release girano con GitHub Actions in questo repository (`.github/workflows/`), avvio manuale:
```
gh workflow run checks.yml  -R KronosHub/Kronos-Remote-Support -f ref=main
git tag vX.Y.Z && git push origin vX.Y.Z
gh workflow run release.yml -R KronosHub/Kronos-Remote-Support -f tag=vX.Y.Z
```
