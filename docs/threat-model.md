# Threat model

## Protected properties

- One configured binding selects one network, INIT txid, and spec hash.
- Invalid active-carrier spends terminate state instead of disappearing.
- Events, counters, object snapshots, and roots depend only on canonical Bitcoin data.
- Mempool observations never affect canonical state.
- Agreement signatures authenticate an exact JCS tuple and release identity.
- A reorg can restore the exact pre-block state without manual inference.

## Trust boundaries

### Bitcoin Core

Core supplies canonical blocks and historical transactions. A compromised node can feed a false chain or omit txindex data. Operate Pipeline B on a node and host that are independent of Pipeline A. Compare block hashes with an additional trusted observer before public agreement publication.

### PostgreSQL

The database can corrupt or reorder state if compromised. The service uses one transaction per block, a single advisory writer lock, foreign keys, fixed hash lengths, and complete inverse journals. Database access control, backups, storage integrity, and host hardening remain operator duties.

### Signer

A stolen Ed25519 seed can forge Pipeline B envelopes but cannot spend Bitcoin carriers or change valid roots. Keep the seed out of images and source control. Publish the verifying key through an authenticated channel. Rotate by changing key id under an explicit incident or release procedure.

### HTTP clients

All endpoints are read only. Responses can expose public chain-derived data. Place rate limits, TLS, authentication where required, and response caching outside the verifier. Do not give the public endpoint direct database credentials.

## Parser risks

- malformed push lengths and trailing bytes
- unknown versions and opcodes
- state sequence overflow
- fee arithmetic underflow or overflow
- same-block prevouts
- incorrect Bitcoin hash byte order
- high-S, non-DER, or non-ALL signatures
- carrier signature order changes
- multiple marker and multiple carrier precedence
- refund relative maturity off by one
- reorgs that remove or invalidate INIT

Unit tests, property tests, fuzz targets, golden vectors, and cross-pipeline tuple comparison cover these areas. They do not replace live chain rehearsals or external review.

## Known operational limits

The current canonical object projection is rewritten inside each block transaction. This favors simple atomic correctness over write efficiency. Load testing must establish an acceptable object count before public launch.

The RPC resolver requires a synced transaction index for historical prevouts. A pruned or partially indexed node is not accepted by readiness.

