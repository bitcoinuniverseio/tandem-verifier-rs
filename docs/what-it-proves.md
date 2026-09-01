# What this verifier proves

This document is the one to read before citing pipeline B as evidence for anything. It states the
exact claim the software makes, and lists every neighbouring claim it does not make.

## The claim

For one Bitcoin block height, pipeline B asserts:

> Using a Rust parser and reducer that share no code with the Tandem indexer, reading blocks and
> historical prevouts from its own Bitcoin Core node, and validating SegWit v0 signatures itself,
> this implementation derived the event root, object-state root, chained root, founding count, total
> object count and active object count listed in the tuple, at exactly this block hash and height,
> under exactly this protocol identifier. The named Ed25519 key signed the RFC 8785 serialization of
> that tuple, and the parser and indexer commits and artifact hashes recorded in it are the ones that
> produced the result.

Nothing more. The signature authenticates the tuple, not the correctness of Tandem.

## Where the independence actually comes from

Independence is not a slogan here, it is a list of things that are physically different between the
two pipelines.

| Dimension | Pipeline A (index-tandem) | Pipeline B (this repository) |
|---|---|---|
| Language | TypeScript on Node | Rust |
| HTTP framework | NestJS on Express | axum |
| Database | MySQL with TypeORM | PostgreSQL with sqlx |
| Parser and reducer | `@bitcoinuniverse/tandem` package | `tandem-core`, written separately |
| Signature verification | Node cryptography | `secp256k1` and `bitcoin` crates |
| Bitcoin Core node | Its own | Its own, operated separately |
| Signing key | Its own key id | Its own key id, in a separate trust map |

The two pipelines have exactly one thing in common: the frozen protocol corpus pinned in
[`../protocol-inputs.lock.json`](../protocol-inputs.lock.json). That is deliberate, and it is also
the limit of the guarantee. See [`protocol-inputs.md`](protocol-inputs.md).

An operator who runs both pipelines on the same host, from the same Bitcoin Core node, under the same
key custody, has kept the code independence and thrown away most of the operational independence. The
threat model treats a single shared node as a single point of failure for both pipelines.

## What it does not prove

**It does not prove the specification is right.** Both pipelines implement the same normative
document. A rule that is wrong, or ambiguous in the same direction to both readers, produces two
matching signatures over the same wrong answer. Agreement detects implementation divergence, not
design error.

**It does not prove the shared vectors are complete.** `verify-vectors` checks five marker
encodings, three event leaves and two object leaves from the published fixture. Passing it means this
implementation agrees with the published corpus on those cases. It does not mean the corpus covers
every path through the reducer.

**It does not prove the chain is real.** Each pipeline believes its own Bitcoin Core node. Two nodes
fed the same false chain agree perfectly. Compare block hashes against an additional trusted observer
before publishing agreement publicly.

**It does not authorize a spend.** A matching pair of tuples is a statement about indexed state at a
height. It is not custody, not a signing authority, and not a guarantee that a carrier output is
safe to spend now. Pipeline A states the same boundary from its side.

**It does not cover the mempool.** Mempool rows live in their own table, are replaced wholesale on
each pass, and never contribute to state, counters or roots. The mempool endpoint is provisional and
carries no agreement.

**It does not prove the database is honest.** The store uses one transaction per block, an advisory
writer lock, foreign keys, fixed hash lengths and complete inverse journals. A compromised PostgreSQL
instance still defeats all of it. Storage integrity, access control and backups stay operator duties.

**It does not prove anything about a live deployment.** Every test in this repository is offline.
There is no recorded evidence here of a live Core session, a live PostgreSQL migration or rollback, a
ZMQ session, a regtest or signet transcript, a cross-pipeline agreement run, a key ceremony or a
timed fuzz campaign. [`launch-gates.md`](launch-gates.md) lists each of these as an open blocking
gate, and [`verification.md`](verification.md) records exactly what was executed locally and when.

**It does not carry mainnet authority.** Mainnet startup requires an explicit boolean, a separately
hashed authorization file bound to the exact protocol id and specification hash, and at least two
named approvers. That is an operational interlock. It does not create launch authority, and the
launch-authority gate is not granted.

## How to use a matched pair correctly

1. Fetch the envelope from both pipelines for the same height.
2. Validate both against `agreement-envelope.schema.json` from the protocol repository.
3. Resolve each `key_id` through an authenticated registry, not through the envelope itself.
4. Reconstruct the RFC 8785 bytes of each `tuple` object and verify each signature over those bytes.
5. Compare the nine semantic fields. A matching height with a different block hash is not agreement.
6. On any mismatch, stop publishing from both pipelines and follow the mismatch procedure in
   [`runbook.md`](runbook.md). Do not pick a winner by ownership or release time.

## Related documents

- Protocol specification and vectors: <https://bitcoinuniverseio.github.io/tandem/>
- Pipeline A: <https://bitcoinuniverseio.github.io/index-tandem/>
- [`agreement.md`](agreement.md) for the envelope this side produces.
- [`threat-model.md`](threat-model.md) for trust boundaries and parser risks.
