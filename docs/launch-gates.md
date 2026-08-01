# Launch gates

All gates are blocking. Mainnet activation stays closed until every row has dated, reviewable evidence.

| Gate | Current state | Required evidence |
|---|---|---|
| Frozen spec and shared corpus | Input hashes locked, local vector check required per release | Signed release tag, artifact hashes, independent vector reports |
| Configured network INIT | Not provided | Confirmed txid, raw transaction, height, namespace, operator approval |
| Rust parser and reducer review | Not completed | Two independent reviewers and resolved findings |
| Core RPC ingestion | Not exercised here | Dedicated-node regtest and signet transcripts |
| PostgreSQL apply and rollback | Not exercised here | Migration, atomic failure, backup, restore, and deep-reorg results |
| ZMQ wakeup | Not exercised here | Disconnect, duplicate, loss, and recovery results with RPC as authority |
| Mempool overlay | Not exercised here | Conflict, eviction, replacement, and restart tests |
| Agreement signer | Unit coverage only | Managed key ceremony, public key publication, rotation drill |
| Pipeline A and B agreement | Not performed | Matching signed tuples for every tested height and reorg branch |
| Load and denial resistance | Not performed | Sustained catch-up and API load report |
| External security review | Not performed | Final report and resolved high-severity findings |
| Monitoring and incident response | Documentation only | Alert exercise and named on-call ownership |
| Mainnet launch authority | Not granted | Explicit recorded authorization |

Do not describe this service as production ready while any row remains incomplete.

The binary adds an operational mainnet interlock. Mainnet startup requires an explicit boolean plus a separately hashed JSON artifact bound to the exact protocol id and spec hash with two named approvers. This interlock does not satisfy or replace the launch authority gate.
