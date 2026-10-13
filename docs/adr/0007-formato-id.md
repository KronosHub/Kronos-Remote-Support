# 0007 — ID numerico a 9 cifre assegnato dal server

**Data:** 2026-10-07 · **Stato:** accettata

## Decisione
Il signaling server assegna a ogni installazione un ID di 9 cifre, legato a una chiave del dispositivo
salvata localmente, così l'ID resta stabile tra i riavvii.

## Conseguenze
- Facile da dettare a voce.
- L'ID non è un segreto: la sicurezza si basa su password + PAKE (vedi ARCHITETTURA.md).
