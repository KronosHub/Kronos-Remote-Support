# 0073 — Cattura "vince l'ultimo fotogramma" e agente a ritmo

**Data:** 2026-11-16 · **Stato:** accettata

## Contesto
Con un computer Windows controllato gli input apparivano dopo circa 1 s anche con latenza di rete di 1 ms; con un
Mac no. Su Windows la cattura passa dall'agente SYSTEM (ADR 0033): l'agente cattura e scrive i fotogrammi BGRA sulla
pipe, l'app li riceve e li passa alla cattura condivisa (ADR 0062), che li distribuisce alle sessioni. Ogni anello
aveva una coda: quando la codifica era più lenta della cattura (la VM ARM codifica in software, ~31 ms a
fotogramma) le code si riempivano e l'encoder lavorava sempre su immagini vecchie.

## Decisione
- **Cattura condivisa**: ogni iscritto ha uno slot con un solo fotogramma (`Mutex<Option<Frame>>` + `Condvar`).
  Il distributore sovrascrive lo slot: chi è lento salta i fotogrammi intermedi e prende sempre il più recente.
  Vale per tutti i sistemi.
- **Agente a ritmo**: `StartCapture.paced` (`serde(default)`). Con `paced` l'agente cattura un fotogramma solo
  dopo che l'app ha confermato il precedente con `Request::FrameDone`, quindi sulla pipe ce n'è al massimo uno.
  `CaptureStarted.paced` dice all'app se l'agente lo supporta; un agente vecchio ignora il campo e continua come
  prima, e l'app non manda `FrameDone`.
- `SERVICE_REVISION` 7 perché il servizio rinnovi la copia protetta dell'agente.

## Conseguenze
Ritardo limitato a un fotogramma per anello invece di code piene. Con codifica lenta il frame rate scende ma
l'immagine resta attuale. Da misurare sulla VM con la 2.3.5.
