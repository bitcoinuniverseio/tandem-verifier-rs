# CLI reference

`tandem-cli` builds a binary called `tandem`. It is an offline verification tool: it never opens a
socket, never reads a database and never talks to Bitcoin Core. Everything it does is a pure function
of files you point it at.

```text
cargo run -q -p tandem-cli -- <command> [options]
```

Every command prints a pretty-printed JSON report on success and exits non-zero with a message on
failure. There are no partial successes.

## `verify-inputs`

Checks that a checked-out Tandem protocol repository is byte for byte the corpus this repository is
pinned to.

| Option | Required | Meaning |
|---|---|---|
| `--lock <path>` | yes | The input lock, normally `protocol-inputs.lock.json`. |
| `--protocol-root <path>` | yes | Root of a checked-out `bitcoinuniverseio/tandem`. |

```text
cargo run -q -p tandem-cli -- verify-inputs --lock protocol-inputs.lock.json --protocol-root ../tandem
```

Report shape:

```json
{
  "schema": "urn:tandem:rust-input-verification-report",
  "filesVerified": 6,
  "hashes": { "<relative path>": "<sha256>" }
}
```

Failures: a lock whose `schema` is not `urn:tandem:rust-protocol-input-lock`, an empty input set, a
path that is absolute or contains anything other than plain components, an unreadable file, or the
first digest mismatch.

## `verify-vectors`

Re-derives the published golden corpus with this repository's own Rust code.

| Option | Required | Meaning |
|---|---|---|
| `--manifest <path>` | yes | `vectors/generated/manifest.json` from the protocol repository. |
| `--spec <path>` | yes | `tandem.md` from the same checkout. |

```text
cargo run -q -p tandem-cli -- verify-vectors --manifest ../tandem/vectors/generated/manifest.json --spec ../tandem/tandem.md
```

It checks the manifest schema and filenames, hashes the specification and the fixture and compares
both against the manifest, recomputes the vector root, then independently derives the protocol
identifier, the namespace, the object key and the object display id, decodes every marker script and
confirms its opcode and payload length, recomputes each event and object leaf preimage, and walks
every declared Merkle level up to the event root, the object-state root and the chained root.

Report shape:

```json
{
  "schema": "urn:tandem:rust-vector-report",
  "specHash": "<sha256>",
  "fixtureSha256": "<sha256>",
  "vectorRoot": "<sha256>",
  "markersVerified": 5,
  "eventLeavesVerified": 3,
  "objectLeavesVerified": 2,
  "eventRoot": "<sha256>",
  "objectStateRoot": "<sha256>",
  "chainedRoot": "<sha256>"
}
```

Against the published protocol tree at commit `af523e21e6232611a9605e46e1782da66579f357`, this
command verified 5 markers, 3 event leaves and 2 object leaves, and produced vector root
`b7f22caf5c9b9f3562f4d842a60a4bb0daa3f2805a5b8ff73e4d716721882c11`.

## `replay`

Runs resolved block views through the independent reducer, without a node or a database. This is the
tool for reproducing a disputed height offline during an agreement mismatch investigation.

| Option | Required | Meaning |
|---|---|---|
| `--input <path>` | yes | A replay file. |

```text
cargo run -q -p tandem-cli -- replay --input replay.json
```

Input shape, using snake_case keys and the public `tandem_core` JSON types:

```json
{
  "binding": {
    "network": "regtest",
    "init_txid": "<32-byte hash, wire order>",
    "spec_hash": "<32-byte hash>"
  },
  "blocks": [
    {
      "hash": "<32-byte hash, wire order>",
      "previous_hash": "<32-byte hash, wire order>",
      "height": 1108,
      "transactions": [
        {
          "txid": "<32-byte hash, wire order>",
          "wtxid": "<32-byte hash, wire order>",
          "version": 2,
          "lock_time": 0,
          "inputs": [
            {
              "prevout": { "txid": "<32-byte hash>", "vout": 0 },
              "sequence": 4294967293,
              "script_sig": "",
              "witness": ["<hex>", "<hex>"],
              "prevout_value": 20000,
              "prevout_script": "<hex>",
              "prevout_height": 1000,
              "signatures_valid": true
            }
          ],
          "outputs": [{ "value": 0, "script_pubkey": "<hex>" }]
        }
      ]
    }
  ],
  "expected_roots": []
}
```

`expected_roots` is optional. When present it must have one entry per block, and the replay aborts at
the first height whose derived roots differ.

Two things about this input are easy to get wrong:

- Every input carries `signatures_valid`, which the reducer trusts. Signature checking is the
  responsibility of whatever produced the replay file. In the running service that is the verifier's
  own SegWit v0 verification step, so a replay file assembled by hand is only as trustworthy as the
  process that filled that field.
- The first block in the list must contain the configured INIT transaction. A file that starts later
  fails with `first processed block does not contain configured INIT`.

Report shape:

```json
{
  "schema": "urn:tandem:rust-replay-report",
  "protocolId": "tndm:regtest:<init-txid>",
  "heights": [
    {
      "height": 1108,
      "block_hash": "<32-byte hash>",
      "event_root": "<sha256>",
      "object_state_root": "<sha256>",
      "chained_root": "<sha256>",
      "counters": { "founding_created": 0, "all_objects": 0, "active_objects": 0 }
    }
  ],
  "eventCounts": [1],
  "finalState": { "...": "the serialized chain state after the last block" }
}
```

`heights` is the same root tuple that ends up in the agreement envelope, so a replay of the disputed
height can be compared directly against what each pipeline signed.

No sample replay corpus ships in this repository, because a real one requires resolved prevout data
from a node. Producing one is part of the regtest and signet transcript work that
[`launch-gates.md`](launch-gates.md) still lists as not exercised.
