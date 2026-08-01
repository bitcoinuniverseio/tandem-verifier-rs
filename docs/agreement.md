# Agreement envelope

`GET /v1/agreement/{height}` reads one persisted canonical root tuple and signs it with Ed25519.

The tuple fields match `agreement-envelope-v1.schema.json`:

- protocol id
- height and counters as unsigned decimal strings
- block hash in Bitcoin display order
- event, object-state, and chained roots as lowercase SHA256 hexadecimal
- parser and indexer commits as 40 lowercase hexadecimal characters
- parser and indexer artifact hashes as 64 lowercase hexadecimal characters

The parser commit, indexer commit, and both binary hashes are stored atomically with the height when it is processed. A later service release does not replace that historical identity.

The signer serializes only the `tuple` object with RFC 8785 JCS. It signs those bytes directly. The envelope signature is 64 raw Ed25519 bytes encoded as 128 lowercase hexadecimal characters.

Signing is unavailable while the worker is catching up, Core is unready, INIT is inactive or failed, the stored tip differs from Core, or Core reports another block hash at the requested height.

Consumers must validate the JSON schema, resolve `key_id` through an authenticated registry, reconstruct the same JCS bytes, verify the signature, and compare all tuple fields. A matching height without a matching block hash is not agreement.
