# Fuzzing

This page exists so that nobody reads the `fuzz/` directory and concludes more than it supports.
There are two `cargo-fuzz` targets. They cover the two places where attacker-controlled bytes reach
pure logic. They cover nothing else, and no campaign result is recorded in this repository.

## What is fuzzed

### `marker`

```text
cargo +nightly fuzz run marker
```

Feeds arbitrary bytes to `find_marker_candidate` as if they were an output script, and calls
`parse()` on any candidate that is detected. It exercises marker detection and the full payload
parser: push length handling, trailing bytes, unknown versions, unknown opcodes and payload length
rules. The property is that no input panics.

The same surface has property coverage in `crates/tandem-core/tests/properties.rs`, which asserts a
stronger invariant on random scripts up to 512 bytes: any successfully parsed marker has a payload
between 7 and 80 bytes, and a recognised opcode always has exactly its declared payload length.

### `replay`

```text
cargo +nightly fuzz run replay
```

Deserializes the input as a JSON array of `BlockView` values and pushes up to 32 of them through
`apply_block` against a fixed regtest binding, stopping at the first reducer error. It exercises the
reducer, the validation ordering, the state sequence arithmetic, the fee arithmetic and the root
computation on structurally valid but semantically hostile blocks.

Two limits matter. The target only reaches the reducer when the fuzzer produces parseable JSON, so
raw byte mutation spends most of its budget on the JSON decoder rather than on Tandem logic. And the
binding is hard coded to a regtest network with dummy INIT and specification hashes, so paths that
depend on a specific configured binding are not explored.

## What is not fuzzed

None of the following has a fuzz target:

- Bitcoin Core JSON-RPC response decoding, including malformed or hostile node responses.
- The historical prevout resolver and SegWit v0 signature verification path.
- The PostgreSQL store: block transactions, the inverse journal, rollback and the projections.
- The HTTP read API, its path and query parsing, and its error mapping.
- The agreement signer, the RFC 8785 serialization and Ed25519 signing.
- The ZMQ wakeup listener.
- CLI argument parsing, the input lock reader and the golden-vector reader.

## What has not been run

No timed fuzz campaign has been executed and recorded for this repository. There is no committed
corpus, no committed crash artifact, no coverage report and no CI job that runs `cargo fuzz`. The
launch-gate table lists external security review as not performed, and fuzzing evidence sits in the
same category.

Treat the two targets as a facility that is ready to use, not as a result that has been obtained.

## Running a campaign

`cargo-fuzz` needs a nightly toolchain, which is not what `rust-toolchain.toml` pins, so it has to be
requested explicitly:

```text
rustup toolchain install nightly
cargo install cargo-fuzz
cargo +nightly fuzz run marker -- -max_total_time=3600
cargo +nightly fuzz run replay -- -max_total_time=3600
```

The `fuzz` crate is excluded from the workspace and carries its own `Cargo.lock`, so it does not
affect a normal `cargo test --workspace` run and it does not pull `libfuzzer-sys` into the release
build.

If a campaign finds a crash, treat it as a security report and follow [`../SECURITY.md`](../SECURITY.md)
rather than opening a public issue. Add the minimized input as a regression test in `tandem-core`
before fixing it, so the case stays covered by the ordinary suite.
