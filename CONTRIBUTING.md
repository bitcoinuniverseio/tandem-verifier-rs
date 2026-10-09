# Contributing

This repository exists to be a second opinion on the Tandem indexer. Anything that erodes its
independence from pipeline A is not an improvement here, however convenient it looks.

## The independence rules

1. Do not add a dependency on the TypeScript Tandem package, on pipeline A's code, or on any crate
   generated from it.
2. Do not read pipeline A's API, database or output anywhere in this workspace.
3. Do not import protocol constants or rules from anywhere except the normative specification. The
   only shared material is the corpus pinned in `protocol-inputs.lock.json`.
4. Do not copy an algorithm from pipeline A to make the two match. If the two disagree, one of them
   has read the specification wrong, and finding out which is the point.

## Before you open a pull request

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --release --locked
```

CI runs exactly these four on a self-hosted Windows runner with the pinned 1.97.1 toolchain. Clippy
is at `deny(all, pedantic)` and `unsafe_code` is forbidden, so a warning is a failure.

If you touched the parser, the reducer or the roots, also run both corpus checks against a checkout
of the protocol repository and paste the reports into the pull request:

```text
cargo run -q -p tandem-cli -- verify-inputs --lock protocol-inputs.lock.json --protocol-root ../tandem
cargo run -q -p tandem-cli -- verify-vectors --manifest ../tandem/vectors/generated/manifest.json --spec ../tandem/tandem.md
```

## Changes that need more than a passing suite

| Change | Also required |
|---|---|
| Consensus behaviour: markers, validation, reason precedence, roots, terminal states | A specification citation for every behavioural claim, plus a regression test for the exact case. |
| The agreement tuple or envelope | Confirmation that the shape still validates against `agreement-envelope.schema.json`, and that pipeline A's nine compared fields are unchanged. |
| The database schema | A migration, an explanation of what happens to an existing bound database, and rollback behaviour. |
| Pinned protocol inputs | The procedure in [`docs/protocol-inputs.md`](docs/protocol-inputs.md), including both CLI reports. |
| A CI workflow | Keep the self-hosted runner label. GitHub-hosted runner labels are not permitted in this organization. |

## Documentation

Documentation lives with the code and is expected to stay true, not aspirational.

- Do not describe an unreleased or unexercised capability as available. If it has not been run, say
  that it has not been run.
- Every claim about behaviour should be traceable to a file in this repository.
- Do not restate the protocol specification. Link to <https://bitcoinuniverseio.github.io/tandem/>.
- Do not use em dashes.
- `docs.manifest.json` must keep validating against the organization documentation manifest schema,
  and `lastVerified.commit` should be updated when the facts in it are re-checked.

## Reporting problems

A parser, signature, root or reorg discrepancy is a security matter. Follow
[`SECURITY.md`](SECURITY.md) and do not open a public issue. Everything else can go to the issue
tracker, and [`SUPPORT.md`](SUPPORT.md) says where.
