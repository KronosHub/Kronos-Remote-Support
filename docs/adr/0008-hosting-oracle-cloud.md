# 0008 — Hosting server su VM Oracle Cloud

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
Signaling server e coturn girano sulla VM Oracle Cloud già disponibile all'utente (docker-compose).

## Note operative (da verificare quando ci arriviamo)
- Aprire le porte in **due** punti: Security List / NSG della VCN in OCI **e** firewall dell'OS
  (le immagini Oracle Linux/Ubuntu di OCI hanno regole iptables restrittive di default).
- Porte previste: 443/tcp (signaling su WSS), 3478/udp+tcp (STUN/TURN), range UDP relay coturn (es. 49152–49252).
- Se la VM è Ampere (ARM64) servono immagini Docker multi-arch o build `aarch64`.
- Serve un nome DNS per il certificato TLS (Let's Encrypt).
