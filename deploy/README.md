# Deploy su VM Oracle Cloud

> Dalla v0.12.0 l'app usa il signaling su Cloudflare Workers (`deploy/cloudflare`, ADR 0035).
> Questa installazione su VM resta disponibile per chi vuole ospitare un proprio server.

Signaling server nativo (binario statico cross-compilato sul Mac) + Caddy per HTTPS/WSS,
entrambi come servizi systemd. Può convivere con altri servizi sulla stessa VM (porte diverse). ADR 0011.

- VM di riferimento: Oracle Linux 9 x86_64, 1 GB RAM + 4 GB swap, SELinux e firewalld attivi;
  utente SSH da `VM_USER` (predefinito `opc`, quello delle immagini Oracle Linux)
- Host pubblico: `<ip-con-trattini>.sslip.io` (risolve a `<IP-DELLA-VM>`)
- Endpoint: `wss://<ip-con-trattini>.sslip.io/ws`, health: `https://<ip-con-trattini>.sslip.io/health`

## Porte
| Porta | Proto | Uso | firewalld | OCI Security List |
|---|---|---|---|---|
| 80 | TCP | ACME HTTP-01 (rinnovo certificato) + redirect | `setup-vm.sh` | da aprire a mano |
| 443 | TCP | WSS signaling | `setup-vm.sh` | da aprire a mano |
| 8080 | TCP | signaling, solo `127.0.0.1` | no | no |
| 3478 | UDP+TCP | STUN/TURN (coturn) | `setup-vm.sh` | da aprire a mano |
| 49160–49200 | UDP | relay TURN | `setup-vm.sh` | da aprire a mano |

OCI: Networking > VCN > Security List della subnet > Add Ingress Rules:
Source CIDR `0.0.0.0/0`, IP Protocol TCP, Destination Port Range `80,443`.
Per il TURN (ADR 0024): TCP `3478`, UDP `3478`, UDP `49160-49200`.

## Prerequisiti sul Mac (una volta)
```
rustup target add x86_64-unknown-linux-musl
brew install zig && cargo install cargo-zigbuild
```

## Deploy / aggiornamento
```
VM_HOST=<IP-DELLA-VM> VM_KEY=~/chiave.pem ./deploy/deploy.sh
```
Lo script: compila, copia, esegue `setup-vm.sh` (idempotente: Caddy con checksum verificato, utente `caddy`,
firewalld http/https), installa binario e unit, riavvia i servizi.

## File installati sulla VM
- `/usr/local/bin/signaling`, `/usr/local/bin/caddy`
- `/etc/systemd/system/signaling.service`, `/etc/systemd/system/caddy.service`
- `/etc/caddy/Caddyfile`
- `/var/lib/signaling/ids.json` (ID dispositivi), `/var/lib/caddy` (certificati)
- `/etc/coturn/turnserver.conf` (0640 root:coturn), `/etc/desktop-remoto/turn-secret` (0600, segreto TURN)

## Avvio automatico
Servizi `enabled` (partono al boot) e con `Restart=on-failure`: `signaling`, `caddy`, `coturn`
(override in `/etc/systemd/system/coturn.service.d/override.conf`).
Regole firewalld permanenti.

## Operazioni
```
sudo systemctl status signaling caddy
sudo journalctl -u signaling -f
sudo journalctl -u caddy -f
```

## Uso dai client
```
cargo run -p peer-poc -- --server wss://<ip-con-trattini>.sslip.io/ws host
cargo run -p peer-poc -- --server wss://<ip-con-trattini>.sslip.io/ws connect <ID>
```
