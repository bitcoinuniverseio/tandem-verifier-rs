# Database model

One PostgreSQL database per verifier, created by `migrations/0001_initial.sql`. There are six tables
and no schema migrations after the first, so the current model is exactly that file.

Every write path takes `pg_advisory_xact_lock(607841294141560657)` first, so block application,
rollback and mempool replacement are serialized against each other even if a second process is
started by mistake.

## Table roles

| Table | Authoritative for | Rebuilt or replaceable |
|---|---|---|
| `protocol_state` | The deployment binding and the serialized Rust reducer state. One row, `id = 1`. | No. Losing it loses the deployment identity. |
| `canonical_blocks` | Per height: block hash, parent hash, the three roots, the three counters, the release identity that produced them, and the complete inverse journal. | No. This is the record the agreement envelope is signed from. |
| `canonical_events` | Query projection of events per height, transaction index, event index and sub index. | Yes, rebuilt inside the same block transaction. |
| `canonical_objects` | Query projection of current object state, keyed by object key. | Yes, rewritten inside each block transaction. |
| `mempool_overlay` | Provisional unconfirmed observations. | Yes, replaced wholesale on each pass. Never an input to state, counters or roots. |
| `reorg_journal` | An audit record of disconnects and restores. | Yes. It is evidence, not consensus input. |

## Integrity rules that live in the schema

The migration does not rely on application discipline alone:

- Every hash column is `bytea` with an `octet_length(...) = 32` check, so a truncated or padded hash
  cannot be stored.
- `parser_commit` and `indexer_commit` are checked against `^[0-9a-f]{40}$`, so release identity
  cannot be written in mixed case or the wrong length.
- Counters are `numeric(20,0)` with non-negative checks, which holds the full unsigned 64-bit range
  without the signed-integer wraparound a `bigint` would allow.
- `canonical_events.height` references `canonical_blocks(height)` with `ON DELETE CASCADE`, so
  removing a disconnected height cannot leave orphaned events behind.
- `canonical_blocks.block_hash` is unique, so the same block cannot be recorded at two heights.
- `protocol_state` is pinned to a single row by `CHECK (id = 1)`.

## The inverse journal

`canonical_blocks.inverse_delta` holds the exact pre-state needed to undo that block. Rollback does
not recompute a previous state from history and it does not infer anything: it restores the recorded
one, deletes the dependent projection rows, writes a `reorg_journal` entry and returns the restored
chain state. That is why a deep reorg is bounded work rather than a full resync, and why losing the
journal is unrecoverable without a resync.

If rollback fails, stop the process and preserve the database. Do not delete state and do not advance
the published agreement height. [`runbook.md`](runbook.md) has the escalation steps.

## Indexes

| Index | Serves |
|---|---|
| `canonical_events_object_key_idx` | Event history for one object, in height and transaction order. |
| `canonical_events_reason_idx` | The invalid-event endpoint, grouped by reason. |
| `canonical_events_txid_idx` | Lookup of events by transaction. |
| `canonical_objects_current_outpoint_idx` | The carrier endpoint, which resolves an outpoint to an object. |
| `canonical_objects_status_idx` | Status and creation-height filtering behind the stats endpoint. |

## Sizing and performance

The object projection is rewritten inside each block transaction. That favours simple atomic
correctness over write efficiency, and it means write cost grows with the number of live objects
rather than with the number of objects touched by the block. It is the known scaling limit of the
current design, it is recorded in [`threat-model.md`](threat-model.md), and load testing must
establish an acceptable object count before any public launch. No measurement exists yet, so this
repository publishes no sizing table.

## Backup and restore

Back up with a tool matched to your recovery objective, and test the restore into a separate
environment. After a restore, compare the latest stored block hash with Core and let the normal reorg
loop reconcile, but only once the backup itself has been preserved. A restore into a database bound
to a different network, INIT txid or specification hash is refused at startup rather than silently
adopted.
