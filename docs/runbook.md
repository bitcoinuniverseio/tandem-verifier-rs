# Operator runbook

## Provision

1. Run a dedicated Bitcoin Core node on the selected network.
2. Enable and fully sync `txindex`.
3. Provision a dedicated PostgreSQL database and role.
4. Check out the exact Tandem specification artifact and verify its SHA256 out of band.
5. Configure one explicit INIT txid. Do not discover INIT by marker order.
6. Create a 32-byte Ed25519 seed with an approved cryptographic key generator. Store it in a restricted file or inject it from the deployment secret manager.
7. Record the parser and indexer commits and release artifact hashes.
8. Start one verifier instance for the database. Do not run concurrent writers.

## Start checks

The process exits before listening when configuration, spec bytes, spec hash, signer seed, database binding, or URL parsing fails. After it listens, `/readyz` remains unavailable until:

- PostgreSQL answers a query.
- Core reports the configured chain.
- Core block and header heights match.
- Core is out of initial block download.
- `txindex` exists and is synced.
- The configured INIT is canonical and valid.
- The ingestion worker has no current error.

## Routine monitoring

Alert on:

- `/readyz` not returning HTTP 200.
- any worker error.
- a `FAILED_INIT` state.
- Core entering initial block download.
- missing or unsynced `txindex`.
- repeated reorg records.
- pipeline agreement mismatch at the same height.
- database storage or connection saturation.
- signer key identifier changing outside an approved release.

## Reorganization response

The worker detects a mismatch by comparing the stored tip hash with Core at the same height. It rolls back one block at a time using the PostgreSQL inverse journal until it reaches a common ancestor. It then applies replacement blocks in height order.

If rollback fails, stop the process and preserve the database. Do not delete state or advance the published agreement height. Collect the current Core best hash, stored tip, last reorg rows, process commit, database backup identifier, and logs. Escalate through the incident process.

## Agreement mismatch response

1. Stop publishing new agreement envelopes from both pipelines.
2. Keep both databases and nodes unchanged.
3. Confirm both tuples use the same protocol id, height, and block hash.
4. Compare event root, object root, counters, and chained root in that order.
5. Replay from the last agreed height using immutable binaries.
6. Treat the first differing event or object leaf as the investigation boundary.
7. Do not designate either pipeline as correct based on ownership or release time.

## Backup and restore

Back up PostgreSQL with a tool appropriate for the selected recovery objective. Test restore into a separate environment. After restore, compare the latest stored block hash to Core and allow the normal reorg loop to reconcile only after the backup has been preserved.

