# Local verification record

Date: 2026-08-01

Environment:

- `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- `cargo 1.97.1 (c980f4866 2026-06-30)`
- Windows MSVC target

The task-local toolchain used `CARGO_HOME=C:\Universe\.codex-tmp\tandem-rust\cargo` and `RUSTUP_HOME=C:\Universe\.codex-tmp\tandem-rust\rustup`.

## Final protocol inputs

The fail-closed `protocol-inputs.lock.json` binds Pipeline B to these exact authoritative artifacts:

- `tandem.md`: `caa77ce0122c0b833fc5f099191b54280b0481be325bdc98f2b48b0b905b923f`
- `schemas/agreement-envelope.schema.json`: `1d5493758b1cc358b02491b675b9e7cb64c51fe3ce2e3f0cde9669882717faa1`
- `schemas/chapter.schema.json`: `9fa613d576b2aeb95b52140c89180f05f65ecd7f4266797f41a1e685610dfc17`
- `schemas/close.schema.json`: `e6645b4ec1eeb44996a37847d4318168959a905fb334b48f4f3298cc6340bc59`
- `vectors/generated/manifest.json`: `d443d9b6e178b95b707620593e471b2146c2747be0f7789dc06f54ce133c33ac`
- `vectors/generated/golden.json`: `fc4bee2c20fe94a66a9849f1dc3d73bc407179474e936de29eddef85dcfb5856`

## Completed commands

`cargo fmt --all -- --check`

- Exit code 0.

`cargo test --workspace --all-features --locked`

- Exit code 0.
- 16 tests passed.
- 0 tests failed.

`cargo run -q -p tandem-cli -- verify-inputs --lock protocol-inputs.lock.json --protocol-root C:\Universe\.codex-tmp\tandem-protocol-audit`

- Exit code 0.
- All 6 shared files matched the Pipeline B lock.

`cargo run -q -p tandem-cli -- verify-vectors --manifest C:\Universe\.codex-tmp\tandem-protocol-audit\vectors\generated\manifest.json --spec C:\Universe\.codex-tmp\tandem-protocol-audit\tandem.md`

- Exit code 0.
- 5 marker encodings verified.
- 3 event leaves verified.
- 2 object leaves verified.
- Vector root: `b7f22caf5c9b9f3562f4d842a60a4bb0daa3f2805a5b8ff73e4d716721882c11`
- Event root: `475b25d221ecaa3abae67c5a4828da0b4cfe752d6cf1b55fa2dff861fae046ce`
- Object-state root: `67ec64ab4c8645ed0d500ea71032c46026254241575979c26b9f7b207e110d79`
- Chained root: `c54cd3c6423a7a35f6fa37ecceed0387e4398c55f8f6a90711c4fb906322e260`

## Scope and limits

This record covers local source, lock, and vector verification. It makes no claim for:

- a live Bitcoin Core RPC session
- a live PostgreSQL migration, block transaction, rollback, backup, or restore
- a live ZMQ notification session
- a Docker image build or Compose startup
- a timed fuzz campaign
- a Pipeline B regtest or signet transcript
- a cross-pipeline signed agreement run
- a production signing-key ceremony
- mainnet INIT or activation authority
