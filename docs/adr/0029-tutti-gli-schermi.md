# 0029 — Tutti gli schermi insieme

**Data:** 2026-10-08 · **Stato:** accettata

## Problema
Richiesta utente: vedere i monitor del partner contemporaneamente, non solo uno alla volta (ADR 0019/multi-monitor).

## Alternative
1. **Più tracce video** (una per monitor): rinegoziazione WebRTC, N encoder e decoder, layout nella UI; complesso.
2. **Composizione lato host** in un'unica immagine: un solo flusso, nessuna modifica a trasporto e decoder, il mouse
   resta una mappatura lineare sull'unione delle aree.

## Decisione
Opzione 2. `capture::ALL_DISPLAYS` (`u32::MAX`) come id di display:
- `capture::composite::CompositeCapturer`: un thread di cattura per monitor (avviati in parallelo: alcuni impiegano
  secondi), ogni frame disegnato sulla tela nella posizione reale (`bounds` in coordinate di input), zone vuote nere.
  Densità = monitor più nitido, poi la tela è ridotta per stare nel limite dell'encoder (3840×2160).
- `bounds()` = unione delle aree → l'iniezione input esistente funziona senza modifiche.
- L'host aggiunge "Tutti gli schermi" alla lista `displays` quando ci sono ≥ 2 monitor; il viewer lo sceglie dal
  selettore esistente (anche le versioni precedenti dell'app lo mostrano, essendo una voce in più).
- Linux X11: la cattura della root window copre già tutti i monitor; nessuna voce aggiuntiva.

## Verifica
Mac con Studio Display (2880×1620 pt) e MacBook (1728×1117 pt, a sinistra in basso): tela 3840×1660, disposizione
verificata su miniatura 16×7 (zone vuote nere dove previsto), 0 risinc. Windows non verificato localmente.

## Limiti
Con molti monitor ognuno è più piccolo (tela max 3840×2160); codifica software di un'immagine grande pesa di più
sulla CPU dell'host. Possibile sviluppo: zoom/scroll nel viewer, encoder hardware.
