# Tandem verifier pipeline B

This repository is an independent Rust implementation of Tandem. It does not import, execute, or depend on the TypeScript parser or indexer.

The only shared protocol inputs are the frozen specification, JSON schemas, and golden vectors listed in `protocol-inputs.lock.json`. Pipeline B resolves Bitcoin data from its own Core RPC boundary, verifies SegWit v0 signatures, reduces state in Rust, stores results in PostgreSQL, and signs the exact agreement tuple with Ed25519 after JCS authoritativeization.

Mainnet activation is closed. This repository contains no configured network INIT, production signing key, live Core evidence, PostgreSQL exercise evidence, ZMQ exercise evidence, regtest transcript, or signet transcript.

## Components

- `tandem-core`: marker parser, reason precedence, transaction validation, reducer, inverse journal, event roots, object roots, and chained roots.
- `tandem-verifier`: Core RPC ingestion, PostgreSQL transactions, reorg recovery, mempool overlay, read API, readiness gates, and agreement signing.
- `tandem-cli`: shared-vector verification and deterministic resolved-block replay.
- `fuzz`: marker and replay fuzz targets for `cargo-fuzz`.

## Build and test

Use the pinned toolchain:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

Verify a checked-out Tandem protocol corpus:

```text
cargo run -p tandem-cli -- verify-inputs \
  --lock protocol-inputs.lock.json \
  --protocol-root ../tandem

cargo run -p tandem-cli -- verify-vectors \
  --manifest ../tandem/vectors/generated/manifest.json \
  --spec ../tandem/tandem.md
```

Replay a resolved block file:

```text
cargo run -p tandem-cli -- replay --input replay.json
```

The replay file contains `binding`, `blocks`, and an optional `expected_roots` array. `blocks` use the public `tandem_core::BlockView` JSON shape. Inputs include exact prevout data and an independently verified signature result.

## Runtime configuration

Every deployment must set these values:

- `TANDEM_NETWORK`
- `TANDEM_INIT_TXID`
- `TANDEM_SPEC_PATH`
- `TANDEM_SPEC_SHA256`
- `DATABASE_URL`
- `BITCOIN_RPC_URL`
- `BITCOIN_RPC_USER`
- `BITCOIN_RPC_PASSWORD`
- `TANDEM_SIGNING_KEY_FILE`
- `TANDEM_SIGNING_KEY_ID`
- `TANDEM_PARSER_COMMIT`
- `TANDEM_INDEXER_COMMIT`
- `TANDEM_PARSER_BINARY_SHA256`
- `TANDEM_INDEXER_BINARY_SHA256`

Mainnet also requires `TANDEM_ALLOW_MAINNET=true`, an authorization JSON file, and the independently supplied SHA256 of that file. The artifact must bind the exact protocol id and spec hash and name at least two approvers. These controls are an operational interlock. They do not create launch authority by themselves.

`TANDEM_INIT_TXID` is required and cannot be zero. The database is permanently bound to the exact network, INIT txid, and spec hash on first startup. A conflicting process fails before ingestion.

The signing key file contains one 32-byte Ed25519 seed as exactly 64 hexadecimal characters. Restrict the file to the service account. Do not place it in source control or an image.

Core must be on the configured network, out of initial block download, at matching block and header heights, and have a synced `txindex`. Readiness stays false otherwise.

## Read API

- `GET /healthz`
- `GET /readyz`
- `GET /tandem/objects/{object_key}`
- `GET /tandem/carriers/{display_txid}/{vout}`
- `GET /tandem/events?height={height}&object_key={object_key}`
- `GET /tandem/invalid`
- `GET /tandem/reorgs`
- `GET /tandem/stats`
- `GET /tandem/mempool`
- `GET /tandem/agreement/{height}`

The mempool endpoint is provisional. Mempool rows never modify authoritative objects, counters, or roots.

Agreement signing fails while the worker is catching up, Core is unready, the protocol is inactive, the stored tip differs from Core, or the requested block hash is no longer authoritative. Source commits and binary hashes are persisted with each block, so a later release cannot relabel historical work.

## Reorganizations

Each block and its complete pre-state are committed in one PostgreSQL transaction. Before a new block is applied, the worker compares its stored tip to Core at the same height. A mismatch restores exact pre-state from the inverse journal, deletes dependent events and roots, records the disconnect, and then applies the replacement branch.

See `docs/runbook.md`, `docs/threat-model.md`, and `docs/launch-gates.md` before operating this service.

## License

MIT
