# Support

## Start with the documentation

| Question | Where the answer is |
|---|---|
| What does this service actually prove? | [`docs/what-it-proves.md`](docs/what-it-proves.md) |
| What are the Tandem rules? | The specification at <https://bitcoinuniverseio.github.io/tandem/> |
| How do I configure it? | [`docs/configuration.md`](docs/configuration.md) |
| Why is `/readyz` returning 503? | The response lists the exact failing gates. [`docs/runbook.md`](docs/runbook.md) explains each one. |
| What do these tables hold? | [`docs/database.md`](docs/database.md) |
| How do I verify the shared corpus? | [`docs/cli.md`](docs/cli.md) |
| What does the signed envelope contain? | [`docs/agreement.md`](docs/agreement.md) |
| Is it fuzzed? | [`docs/fuzzing.md`](docs/fuzzing.md), including what is not |
| Can I run this on mainnet? | [`docs/launch-gates.md`](docs/launch-gates.md). Not yet. |

## Where to ask

| Topic | Where |
|---|---|
| A bug, a wrong behaviour, or a documentation error in this repository | Issues on [bitcoinuniverseio/tandem-verifier-rs](https://github.com/bitcoinuniverseio/tandem-verifier-rs/issues) |
| A protocol rule, a specification ambiguity, or a test vector | [bitcoinuniverseio/tandem](https://github.com/bitcoinuniverseio/tandem/issues) |
| The indexer, its API, or its verified surface | [bitcoinuniverseio/index-tandem](https://github.com/bitcoinuniverseio/index-tandem/issues) |
| A vulnerability, or any parser, signature, root or reorg discrepancy | Private disclosure per [`SECURITY.md`](SECURITY.md). Never a public issue. |

## What to include in a report

- The commit you are running, and the network.
- The configured protocol identifier, which `GET /readyz` returns.
- The exact `/readyz` body when readiness is the problem.
- The block height and block hash where the behaviour differs.
- Whether the two pipelines disagree, and on which of the nine compared fields.
- Reproduction steps, ideally as a `replay` input file.

Never include private keys, signing seeds, RPC credentials, database credentials, wallet descriptors
or user transaction data.

## What is out of scope

This repository does not provide wallet support, custody, trading, or advice about spending. Tandem
has no marketplace entry in the published Bitcoin Universe capability snapshot, so there is no
Universe listing, buying, offer or settlement path to support.
