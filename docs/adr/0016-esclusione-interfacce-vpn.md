# 0016 — ICE: esclusione delle interfacce VPN/tunnel

**Data:** 2026-10-07 · **Stato:** accettata

## Contesto
Tra due istanze sullo stesso Mac il RTT del canale dati era 12–150 ms invece di <1 ms. Causa: interfaccia
`utun8` (Tailscale, 100.84.x.x); ICE a volte nominava la coppia di candidati via Tailscale (relay DERP).

## Decisione
I socket UDP locali si aprono solo sulle interfacce IPv4 fisiche (crate `if-addrs`), escludendo nomi che
iniziano con `utun`, `tun`, `tap`, `wg`, `tailscale`, `zt`, `zerotier`, `ppp`, `ipsec`, `wireguard`, `openvpn`
o contengono `vpn`. Se non ne resta nessuna si usa `0.0.0.0` (fallback srflx/relay).

## Risultato
RTT a regime 0,2 ms (solo il primo ping, durante avvio stream e assestamento ICE, è più alto).

## Conseguenze
- Due PC raggiungibili *solo* tramite VPN non usano più quel percorso diretto: passeranno da srflx o TURN (Fase 5).
- La libreria (webrtc 0.21) non applica il filtro interfacce del `SettingEngine` nel livello async: lo facciamo noi
  passando indirizzi espliciti (perdiamo la ri-espansione automatica al riavvio ICE).
