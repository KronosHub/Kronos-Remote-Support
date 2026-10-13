# 0011 — Deploy nativo con systemd (niente Docker)

**Data:** 2026-10-07 · **Stato:** accettata (sostituisce il piano Docker dell'ADR 0010)

## Contesto
VM Oracle Linux 9.8 x86_64 con 945 MiB di RAM (~410 MiB liberi) e 4 GB di swap, già usata da RustDesk
(hbbs/hbbr nativi + systemd). Docker non installato. Compilare Rust sulla VM sarebbe lentissimo.

## Decisione
- Cross-compilazione sul Mac: `cargo zigbuild --target x86_64-unknown-linux-musl` → binario statico (~3 MB).
- Signaling come servizio systemd con `DynamicUser`, hardening e `MemoryMax=64M`, in ascolto solo su `127.0.0.1:8080`.
- Caddy: binario ufficiale (checksum verificato) + unit systemd, `MemoryMax=128M`.
- Script `deploy/deploy.sh` (dal Mac) + `deploy/setup-vm.sh` (sulla VM, idempotente).

## Conseguenze
- Niente demone Docker (~100 MB risparmiati), stesso stile operativo di RustDesk.
- SELinux: file installati con `install` + `restorecon` per avere le etichette corrette.
- Serve toolchain sul Mac: target musl, zig, cargo-zigbuild.
