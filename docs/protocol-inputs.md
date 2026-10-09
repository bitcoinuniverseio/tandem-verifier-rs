# Pinned protocol inputs

Pipeline B is an independent implementation, but it is not an independent protocol. It has to read
the same normative rules as the indexer. `protocol-inputs.lock.json` is the file that makes that
sharing explicit, bounded and checkable instead of implicit.

## What the lock file contains

```json
{
  "schema": "urn:tandem:rust-protocol-input-lock",
  "sourceRepository": "https://github.com/bitcoinuniverseio/tandem",
  "inputs": {
    "<repository-relative path>": "<sha256 of the exact file bytes>"
  }
}
```

Six files are pinned, and they are the complete set of material this repository takes from anywhere
else:

| Pinned file | What it is |
|---|---|
| `tandem.md` | The normative specification. Its SHA256 is also the `spec_hash` in the deployment binding. |
| `schemas/agreement-envelope.schema.json` | The shape consumers validate a signed envelope against. |
| `schemas/chapter.schema.json` | The chapter manifest schema. Strict, named and non-consensus. |
| `schemas/close.schema.json` | The close manifest schema. Strict, named and non-consensus. |
| `vectors/generated/manifest.json` | The golden-vector manifest, carrying the spec hash, fixture name, fixture digest and vector root. |
| `vectors/generated/golden.json` | The golden fixture itself: marker encodings, identity derivations, event and object leaves, and every Merkle level. |

No source code, no build artifact and no runtime dependency crosses from the protocol repository or
from pipeline A into this one. `Cargo.toml` has no dependency on any Tandem package.

## How the check runs

```text
cargo run -q -p tandem-cli -- verify-inputs --lock protocol-inputs.lock.json --protocol-root ../tandem
```

The command reads the lock, rejects any lock whose `schema` is not
`urn:tandem:rust-protocol-input-lock`, rejects an empty input set, and rejects any path that is
absolute or contains a component other than a plain name, so a lock file cannot be used to read
outside the protocol root. It then hashes each pinned file with SHA256 and fails on the first
mismatch. Success prints every path with the digest it actually observed:

```json
{
  "schema": "urn:tandem:rust-input-verification-report",
  "filesVerified": 6,
  "hashes": {
    "schemas/agreement-envelope.schema.json": "1d5493758b1cc358b02491b675b9e7cb64c51fe3ce2e3f0cde9669882717faa1",
    "schemas/chapter.schema.json": "9fa613d576b2aeb95b52140c89180f05f65ecd7f4266797f41a1e685610dfc17",
    "schemas/close.schema.json": "e6645b4ec1eeb44996a37847d4318168959a905fb334b48f4f3298cc6340bc59",
    "tandem.md": "caa77ce0122c0b833fc5f099191b54280b0481be325bdc98f2b48b0b905b923f",
    "vectors/generated/golden.json": "fc4bee2c20fe94a66a9849f1dc3d73bc407179474e936de29eddef85dcfb5856",
    "vectors/generated/manifest.json": "d443d9b6e178b95b707620593e471b2146c2747be0f7789dc06f54ce133c33ac"
  }
}
```

The digests above were reproduced on 2026-09-01 against
[bitcoinuniverseio/tandem](https://github.com/bitcoinuniverseio/tandem) at commit
`af523e21e6232611a9605e46e1782da66579f357`, which is the currently published protocol tree. All six
matched.

## The specification hash is enforced twice

`verify-inputs` is a development check. The running service enforces the same binding independently:

- `TANDEM_SPEC_PATH` must point at a readable specification file.
- Its bytes are checked against an exact byte contract before they are hashed: no UTF-8 byte order
  mark, valid UTF-8, no carriage return anywhere, exactly one trailing LF, and no trailing space or
  tab on any line. A CRLF copy is rejected with that message rather than surfacing later as an
  unexplained hash mismatch. Unit tests cover both the accepting and the rejecting direction.
- The bytes are then hashed and must equal `TANDEM_SPEC_SHA256`.
- The resulting hash becomes part of the deployment binding, alongside the network and the INIT txid,
  and the binding is written into the database on first startup and never allowed to change.

So a deployment cannot silently drift onto a different specification, and it cannot be pointed at a
database that was built under a different one.

## What the lock does not guarantee

- It pins bytes, not meaning. Two implementations reading the same pinned specification can still
  read an ambiguous sentence the same wrong way.
- It does not verify the protocol repository's provenance. Verify the commit and any release
  signature out of band before trusting a checkout.
- It does not cover completeness. The pinned vectors are the published corpus, not a proof that every
  reducer path is exercised.
- It does not update itself. When the protocol repository publishes new bytes, this repository needs
  a deliberate commit that updates the digests and re-runs both CLI checks. A stale lock fails loudly
  rather than drifting.

## Updating the lock

1. Check out the exact protocol commit you intend to bind to and record it.
2. Recompute the six digests from that checkout.
3. Update `protocol-inputs.lock.json` and, if the specification changed, `TANDEM_SPEC_SHA256` in
   `.env.example` and in every deployment.
4. Run `verify-inputs` and `verify-vectors` and paste both reports into the change.
5. Remember that an existing database is bound to the old specification hash. A specification change
   is a new deployment, not a migration.
