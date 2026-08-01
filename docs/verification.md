# Local verification record

Date: 2026-08-01

Environment:

- `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- `cargo 1.97.1 (c980f4866 2026-06-30)`
- Windows MSVC target

The task-local toolchain used `CARGO_HOME=C:\Universe\.codex-tmp\tandem-rust\cargo` and `RUSTUP_HOME=C:\Universe\.codex-tmp\tandem-rust\rustup`.

## Completed commands

`cargo fmt --all -- --check`

- Exit code 0.

`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`

- Exit code 0.
- No warnings.

`cargo test --workspace --all-features --locked`

- Exit code 0.
- 16 tests passed.
- 0 tests failed.
- Unit, lifecycle, mainnet-interlock, property, and documentation test binaries completed.

`cargo check --manifest-path fuzz\Cargo.toml --bins`

- Exit code 0.
- Both `marker` and `replay` fuzz targets compiled.
- No timed fuzz campaign was run.

`cargo build --workspace --all-features --release --locked`

- Exit code 0.
- Final optimized build completed in 4 minutes 18 seconds.
- `tandem-verifier.exe` SHA256: `dd78170711732ee3f6b184648337831aa761d025aee2ae2d865676ee45f76dba`
- `tandem-cli.exe` SHA256: `debd5189c36304f18ccc3df3b0d903ef6490a8a4b989f6c5dcbb8fcebcc47db9`

`cargo run -q -p tandem-cli -- verify-inputs --lock protocol-inputs.lock.json --protocol-root C:\Universe\work\tandem`

- Exit code 0.
- 6 shared files matched the Pipeline B lock.
- Specification SHA256: `912e8ebad7eeb40c86734724962e4c8b9ba27d248600ff93fcb2b5bf9efc2167`
- Golden fixture SHA256: `3c16004012fa0e6ebe5ab959b90bf947c43d86b281bc794ca41f75e7566aebd2`

`cargo run -q -p tandem-cli -- verify-vectors --manifest C:\Universe\work\tandem\vectors\generated\manifest.json --spec C:\Universe\work\tandem\tandem-v1.md`

- Exit code 0.
- 5 marker encodings verified.
- 3 event leaves verified.
- 2 object leaves verified.
- Vector root: `76379d2ca5f95b4d27860ebcd2ff04d309e4cac27516bac0e689775ed78605d4`
- Event root: `c053efb95b667ba105d91980316ab207962658a22642f02c142c5251bc159863`
- Object-state root: `f4f89a58d050d50ddc50d5fd1e2801594380c0375585e9062605d73e13f02740`
- Chained root: `a16f1d04ddae31bdc3d1cab4f1402f19f523117fca9302c2cf053f9551061f33`

## Evidence not produced

No claim is made for:

- a live Bitcoin Core RPC session
- a live PostgreSQL migration, block transaction, rollback, backup, or restore
- a live ZMQ notification session
- a Docker image build or Compose startup
- a timed fuzz campaign
- a Pipeline B regtest transcript
- a Pipeline B signet transcript
- a cross-pipeline signed agreement run
- a production signing-key ceremony
- mainnet INIT or activation authority

## Candidate release identity

The optimized binaries above were rebuilt from immutable source commit `ca8fdd932b06e506915b42aa4d0a9e4fb0555ed6` with a clean tracked tree and the locked dependency graph.

- Parser source commit: `ca8fdd932b06e506915b42aa4d0a9e4fb0555ed6`
- Indexer source commit: `ca8fdd932b06e506915b42aa4d0a9e4fb0555ed6`
- Parser verification artifact, `tandem-cli.exe`: `debd5189c36304f18ccc3df3b0d903ef6490a8a4b989f6c5dcbb8fcebcc47db9`
- Indexer service artifact, `tandem-verifier.exe`: `dd78170711732ee3f6b184648337831aa761d025aee2ae2d865676ee45f76dba`

This identity is a local candidate release record. It is not a signed release, reproducible-build attestation, production deployment, or mainnet authorization.
