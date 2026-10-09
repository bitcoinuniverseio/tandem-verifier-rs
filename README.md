# Tandem verifier (pipeline B)

An independent Rust implementation of the Tandem protocol, written to disagree with the Tandem
indexer when the indexer is wrong.

Tandem records are only valid as a matched pair of Bitcoin transactions. The normative protocol
specification, JSON Schemas and test vectors live in
[bitcoinuniverseio/tandem](https://github.com/bitcoinuniverseio/tandem) and are published at
<https://bitcoinuniverseio.github.io/tandem/>. This repository does not restate them. It reads them,
pins them by hash, and implements them a second time.

- Protocol specification and vectors: <https://bitcoinuniverseio.github.io/tandem/>
- Indexer (pipeline A), TypeScript and MySQL: [bitcoinuniverseio/index-tandem](https://github.com/bitcoinuniverseio/index-tandem)
- Indexer documentation: <https://bitcoinuniverseio.github.io/index-tandem/>

## Why a second implementation exists

Pipeline A is TypeScript, NestJS and MySQL. Pipeline B is Rust, axum and PostgreSQL. They share no
source code, no parser, no reducer, no database engine and no runtime. The only material both sides
consume is the frozen protocol corpus listed in `protocol-inputs.lock.json`.

That separation is the entire point. A bug in the TypeScript parser, the MySQL schema, the Node
runtime, or one team's reading of an ambiguous rule will not reproduce identically in an independent
Rust implementation reading the same specification. When both pipelines independently reduce the same
Bitcoin height and sign the same tuple of roots and counters, a reader has evidence that is stronger
than either implementation on its own.

```text
             Bitcoin Core (node A)              Bitcoin Core (node B)
                      |                                  |
          TypeScript parser and reducer        Rust parser and reducer
                      |                                  |
                    MySQL                            PostgreSQL
                      |                                  |
          signed agreement tuple A            signed agreement tuple B
                       \                                /
                        compare 9 semantic fields at one height
                                        |
                       serve the answer, or refuse to answer
```

Agreement is compared, never merged. Pipeline B never reads pipeline A's output, and pipeline A never
imports pipeline B's code. Both sign, and the consumer compares.

## What this verifier proves, and what it does not

Read [`docs/what-it-proves.md`](docs/what-it-proves.md) before quoting this service as evidence. The
short form:

It proves that a separate implementation, in a separate language, on a separate database, reading a
separately configured Bitcoin Core node, derived the same Tandem event root, object-state root,
chained root and object counters at an exact block hash and height, and signed that statement with a
named Ed25519 key.

It does not prove that the Tandem specification is correct, that the shared vectors are complete,
that the Bitcoin chain either node reports is the real chain, or that a matching pair of tuples makes
a spend safe. It also proves nothing about mainnet, because mainnet activation is closed here and no
live chain, database or key-ceremony evidence has been recorded. See
[`docs/launch-gates.md`](docs/launch-gates.md) for the exact open gates.

## Product status

Tandem has no entry in the published Bitcoin Universe capability snapshot, which records per protocol
which Universe surfaces implement which actions. No Universe product implements a listing, buying,
offer or settlement path for Tandem. This repository is protocol infrastructure, not a marketplace
component.

## Components

| Crate | Responsibility |
|---|---|
| `tandem-core` | Marker parser, reason precedence, transaction validation, reducer, inverse journal, event roots, object roots, chained roots. No I/O, no database, no network. |
| `tandem-verifier` | Bitcoin Core RPC ingestion, PostgreSQL block transactions, reorg recovery, mempool overlay, read API, readiness gates, agreement signing. |
| `tandem-cli` | Shared-input hash verification, golden-vector verification, deterministic resolved-block replay. |
| `fuzz` | Two `cargo-fuzz` targets over the parser and the reducer. See [`docs/fuzzing.md`](docs/fuzzing.md). |

## System requirements

- Rust 1.97.1, pinned by `rust-toolchain.toml`. `rustup` installs it on the first build.
- PostgreSQL 14 or later, one dedicated database, one writer process.
- Bitcoin Core on the configured network, out of initial block download, with a fully synced
  `txindex`. Pruned and partially indexed nodes are rejected by readiness.
- The release profile sets `lto = true`, `codegen-units = 1` and `panic = "abort"`, so a release
  build links slowly. Budget for it.

`unsafe_code` is forbidden across the workspace and clippy runs at `deny(all, pedantic)`.

## Build and test

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --release
```

The suite is 16 tests: 7 unit tests in `tandem-core`, 3 property tests in
`crates/tandem-core/tests/properties.rs`, and 6 unit tests in `tandem-verifier` covering the mainnet
interlock, the exact line-ending contract on the specification file, txid byte order, the Core
`txindex` gate and stable JCS signatures. None of them touch a database, a node or the network, so a
green suite says nothing about a deployment.

## Verify the shared protocol corpus

These two commands are the reason the CLI exists. They check that this repository is bound to exactly
the protocol artifacts it claims, and that its independent Rust code reproduces the published vector
roots. Clone the protocol repository next to this one and run:

```text
git clone https://github.com/bitcoinuniverseio/tandem.git ../tandem

cargo run -q -p tandem-cli -- verify-inputs --lock protocol-inputs.lock.json --protocol-root ../tandem

cargo run -q -p tandem-cli -- verify-vectors --manifest ../tandem/vectors/generated/manifest.json --spec ../tandem/tandem.md
```

`verify-inputs` hashes all six pinned files and fails on the first mismatch. `verify-vectors`
re-derives the protocol identifier, namespace, object key, five marker encodings, three event leaves,
two object leaves and every Merkle level from the published fixture, using this repository's own
code. Both print a JSON report and exit non-zero on any mismatch.

See [`docs/cli.md`](docs/cli.md) for the exact output shapes and
[`docs/protocol-inputs.md`](docs/protocol-inputs.md) for what the lock file does and does not
guarantee.

## Run it against real data

1. Provision Bitcoin Core and PostgreSQL as described in [`docs/runbook.md`](docs/runbook.md).
2. Apply `migrations/0001_initial.sql`. The schema is documented in
   [`docs/database.md`](docs/database.md).
3. Copy `.env.example` to `.env` and set every value. Every variable is described in
   [`docs/configuration.md`](docs/configuration.md).
4. Start the verifier. It refuses to listen if the binding, specification bytes, specification hash,
   signing seed, database binding or any URL fails validation.
5. Poll `GET /readyz` until it returns HTTP 200. It stays at HTTP 503 and lists the exact failing
   gates until PostgreSQL answers, Core reports the configured chain, Core block and header heights
   match, Core is out of initial block download, `txindex` is synced, the configured INIT is
   authoritative and valid, the worker has no current error, and the stored tip equals Core's height.

The database is permanently bound to the network, INIT txid and specification hash on first startup.
A process configured differently fails before it ingests anything.

Initial synchronization starts at the height of the configured INIT transaction, not at the genesis
block. Until INIT confirms, the worker logs that the configured INIT is not authoritative yet, keeps
the mempool overlay fresh and applies nothing. After INIT confirms it applies every block from that
height to Core's tip, one PostgreSQL transaction per block, then follows the tip. Duration and
resource use depend on the deployment height and block contents, and no timed measurement has been
recorded for any network, so this repository states none.

## Read API

Routes follow the organization convention of serving a protocol under its own prefix, so every Tandem
route is under `/tandem/`. Pipeline A uses the same prefix.

| Route | Purpose |
|---|---|
| `GET /healthz` | Process liveness only. Never gate traffic on it. |
| `GET /readyz` | Every readiness gate, with the failing gates listed on HTTP 503. |
| `GET /tandem/objects/{object_key}` | One object by 32-byte key. |
| `GET /tandem/carriers/{display_txid}/{vout}` | The object holding a carrier outpoint. |
| `GET /tandem/events?height=&object_key=` | Events, filtered by height or object. |
| `GET /tandem/invalid` | Events recorded as invalid, with their reason. |
| `GET /tandem/reorgs` | The reorg journal. |
| `GET /tandem/stats` | Tip height and object counters. |
| `GET /tandem/mempool` | Provisional overlay. Never affects state, counters or roots. |
| `GET /tandem/agreement/{height}` | The signed agreement envelope for one height. |

Every route is read only. Put TLS, authentication, rate limiting and caching in front of the service,
and never give it public database credentials.

## How pipeline A consumes this service

Pipeline A fetches `{PIPELINE_B_BASE_URL}/agreement/{height}`. Because this service serves the
envelope at `/tandem/agreement/{height}`, pipeline A must be configured with the `/tandem` path
segment included, for example `http://verifier.internal:8088/tandem`. Pointing it at the bare origin
produces a 404 on every attempt, which pipeline A reports to its own callers as
`verification_unavailable`.

Pipeline A then verifies both signatures against separate trust maps and compares nine semantic
fields: `protocol_id`, `height`, `block_hash`, `event_root`, `object_state_root`, `chained_root`,
`founding_created`, `all_objects` and `active_objects`. The four release identity fields are signed
and returned but deliberately not compared, because two independent implementations are expected to
be different code. [`docs/agreement.md`](docs/agreement.md) describes the envelope this side
produces.

## Reorganizations and mempool

Each block and its complete pre-state are committed in one PostgreSQL transaction under an advisory
lock. Before a new block is applied, the worker compares its stored tip to Core at the same height. A
mismatch restores exact pre-state from the inverse journal, deletes dependent events and roots,
records the disconnect, and then applies the replacement branch. Rollback repeats one block at a time
until the stored tip matches Core or the protocol returns to its pre-INIT state.

The mempool overlay is a separate table, replaced wholesale on each pass. It never contributes to
state, counters or roots, and the endpoint that exposes it is provisional.

ZMQ `rawblock` is optional and is only a wakeup. The worker discards the payload and refetches the
block through RPC, so a lost or reordered notification cannot become a consensus input.

## Documentation

| Document | Contents |
|---|---|
| [`docs/what-it-proves.md`](docs/what-it-proves.md) | The exact claim, and every claim this service does not make. |
| [`docs/architecture.md`](docs/architecture.md) | Boundaries, data ownership, and why ZMQ is only a wakeup. |
| [`docs/protocol-inputs.md`](docs/protocol-inputs.md) | How `protocol-inputs.lock.json` pins the shared corpus. |
| [`docs/configuration.md`](docs/configuration.md) | Every environment variable, with validation rules. |
| [`docs/database.md`](docs/database.md) | Table-by-table schema and what each table is authoritative for. |
| [`docs/cli.md`](docs/cli.md) | The `tandem` CLI reference and JSON report shapes. |
| [`docs/agreement.md`](docs/agreement.md) | The signed tuple, its JCS bytes, and consumer duties. |
| [`docs/fuzzing.md`](docs/fuzzing.md) | What is fuzzed, what is not, and what has not been run. |
| [`docs/threat-model.md`](docs/threat-model.md) | Protected properties, trust boundaries, parser risks. |
| [`docs/runbook.md`](docs/runbook.md) | Provisioning, monitoring, reorg and mismatch response, backups. |
| [`docs/launch-gates.md`](docs/launch-gates.md) | Every blocking gate before mainnet, and its state. |
| [`docs/verification.md`](docs/verification.md) | A dated local verification record and its stated limits. |

## Contributing, support and security

- [`CONTRIBUTING.md`](CONTRIBUTING.md) for the review bar and the checks that must pass.
- [`SUPPORT.md`](SUPPORT.md) for where questions go.
- [`SECURITY.md`](SECURITY.md) for private vulnerability reporting. Do not open a public issue for a
  parser, signature, root or reorg discrepancy.

## Versioning and releases

The workspace is at `0.1.0` and no Universe release tag exists yet, so `docs.manifest.json` records
this repository as `experimental`. Release identity is not the crate version: every persisted height
stores the parser commit, indexer commit and both artifact hashes that produced it, so a later
release cannot relabel historical work.

## License

MIT. See [`LICENSE`](LICENSE).
