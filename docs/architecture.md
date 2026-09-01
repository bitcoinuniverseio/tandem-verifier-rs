# Architecture

Pipeline B is the independent Rust implementation of Tandem. The protocol rules it implements are
published at <https://bitcoinuniverseio.github.io/tandem/> and are not restated here.
[`what-it-proves.md`](what-it-proves.md) states the claim this architecture is built to support, and
[`database.md`](database.md) covers the storage model in detail.

Pipeline B has separate consensus, ingestion, persistence, and presentation boundaries.

```text
Bitcoin Core RPC
  -> raw block and historical prevout resolver
  -> independent SegWit v0 signature verifier
  -> tandem-core parser and reducer
  -> one PostgreSQL block transaction
  -> read API and JCS plus Ed25519 agreement envelope

Bitcoin Core mempool
  -> separate provisional PostgreSQL overlay
  -> mempool read endpoint only
```

ZMQ is optional. A `rawblock` message is only a wakeup. The worker discards its payload and fetches the authoritative block through RPC. This prevents notification order or message loss from becoming consensus input.

## Data ownership

`protocol_state` stores the authoritative serialized Rust reducer state. `canonical_blocks` stores roots, the exact parser and indexer release identity, and the exact inverse block journal. `canonical_events` and `canonical_objects` are query projections rebuilt inside the same transaction. `mempool_overlay` is independent and replaceable. `reorg_journal` is an audit record, not a consensus input.

Pipeline A output is never read. Agreement occurs only by comparing signed tuples after both pipelines independently process the same authoritative height.
