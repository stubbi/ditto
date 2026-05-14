# ditto-memory

`MemoryController` + `Storage` trait for Ditto.

## Architecture

```
                ┌─────────────────────────────┐
   agent ─►──── │      MemoryController       │ ──► single-writer commit
                │  - content addressing       │       │
                │  - hash chain per source    │       ▼
                │  - signed receipts          │     ┌──────────┐
                │  - idempotency check        │     │ Storage  │
                └─────────────────────────────┘     └──────────┘
```

The controller is the only writer. Agents emit payloads; the controller computes the event_id, mints a signed receipt, and hands off to storage in one transaction.

## Storage backends

- `InMemoryStorage` — reference implementation. Naive substring search. Used for tests and as the control-floor backend.
- `ditto-storage-postgres::PostgresStorage` — production backend (separate crate).
- SQLite + sqlite-vec embedded mode — forthcoming.

## What's not yet implemented (forthcoming)

- Surprise-gated writes (encoder prediction-error)
- Reconsolidation labile window on retrieval
- Metacognitive retrieval gate (RSCB-MC)
- Awake ripple / dream cycle / long sleep consolidation
- RL-trained operations policy (Memory-R1 / Mem-α lineage)
- Explainable retrieval API
- Verifiable cascade deletion

See [`docs/architecture/memory.md`](../../docs/architecture/memory.md) for the full v2 spec.
