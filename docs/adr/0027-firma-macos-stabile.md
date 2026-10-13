# 0027 — Firma macOS con identità stabile (autofirmata)

**Data:** 2026-10-08 · **Stato:** accettata (richiesta utente: "continuo a dover approvare i permessi")

## Problema
Con la firma ad-hoc ogni build ha un'identità diversa: macOS (TCC) dimentica Registrazione schermo e
Accessibilità a ogni installazione/aggiornamento.

## Decisione
- Certificato di firma del codice **autofirmato** "Kronos Remote Support Signing" (RSA 3072, 10 anni,
  EKU codeSigning), creato con openssl e importato nel portachiavi di accesso del Mac di sviluppo.
  File in `~/.tauri/codesign/` (0600, fuori dal repo): `signing.p12`, `p12.password`, `key.pem`, `cert.pem`.
- Requisito designato risultante: `identifier "tech.kronos.desktopremoto" and certificate root = H"27397ac3…"`:
  stabile tra le build, quindi i permessi restano tra un aggiornamento e l'altro.
- CI (repo pubblico): secret `APPLE_CERTIFICATE` (p12 base64), `APPLE_CERTIFICATE_PASSWORD`,
  `APPLE_SIGNING_IDENTITY`, `KEYCHAIN_PASSWORD`. Il certificato viene importato **a mano** in un portachiavi
  temporaneo e a Tauri si passa solo `APPLE_SIGNING_IDENTITY`: l'import integrato di Tauri cerca solo identità
  attendibili (`find-identity -v`) e fallisce con "failed to resolve signing identity" (primo tentativo 0.7.5).
- Vale per **tutti i Mac**: stessa identità su ogni copia, permessi concessi una volta per Mac.
- Build locali: `APPLE_SIGNING_IDENTITY="Kronos Remote Support Signing"`.

## Limiti
- Non è un Developer ID e non c'è notarizzazione: Gatekeeper avvisa ancora al primo download su altri Mac
  (workaround `xattr` nel README). Per eliminarlo: Apple Developer Program.
- Il passaggio da ad-hoc a firmata richiede di concedere i permessi **un'ultima volta**.
- Se il certificato va perso, basta crearne uno nuovo: si riconcedono i permessi una volta.
