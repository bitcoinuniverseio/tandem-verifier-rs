# Configuration

Every setting is a command line flag with an environment variable of the same meaning, parsed by
`clap` in `crates/tandem-verifier/src/config.rs`. `.env.example` lists them all with placeholder
values. Configuration is validated once, before the process listens. A failure exits rather than
starting in a degraded state.

## Protocol binding

These three values, together, are the deployment identity. They are written into `protocol_state` on
first startup and can never change for that database.

| Variable | Required | Rule |
|---|---|---|
| `TANDEM_NETWORK` | yes | One of `mainnet`, `signet`, `testnet4`, `regtest`. Selects the network byte used in markers and in the namespace commitment, and the chain identity readiness expects from Bitcoin Core. There is no plain `testnet`. |
| `TANDEM_INIT_TXID` | yes | The configured INIT transaction id in display order, exactly 64 hexadecimal characters, and not all zeroes. INIT is never discovered by scanning: a different txid is a different protocol. |
| `TANDEM_SPEC_PATH` | yes | Path to the exact normative specification file. Mount it read only. |
| `TANDEM_SPEC_SHA256` | yes | SHA256 of those exact bytes, 64 lowercase hexadecimal characters. |

The protocol identifier is derived, not configured. It has the form `tndm:<network>:<init-txid>`.

Example for a regtest deployment:

```text
TANDEM_NETWORK=regtest
TANDEM_INIT_TXID=8f3c1d0b5a2e47698c0d1f2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e
TANDEM_SPEC_PATH=/protocol/tandem.md
TANDEM_SPEC_SHA256=caa77ce0122c0b833fc5f099191b54280b0481be325bdc98f2b48b0b905b923f
```

The specification hash above is the digest of the currently published `tandem.md`. The INIT txid is
an illustrative placeholder: there is no configured INIT for any network in this repository.

## Storage

| Variable | Required | Rule |
|---|---|---|
| `DATABASE_URL` | yes | PostgreSQL connection URL. It must begin with `postgres://` or `postgresql://`, and any other scheme is rejected at startup. One dedicated database, one role, one writer process. |

Run exactly one verifier per database. The store takes a PostgreSQL advisory transaction lock on
every write path, so a second writer will block rather than corrupt, but concurrent writers are not a
supported configuration.

## Bitcoin Core

| Variable | Required | Rule |
|---|---|---|
| `BITCOIN_RPC_URL` | yes | RPC endpoint of a dedicated node on the configured network. Must parse as a URL with an `http` or `https` scheme. |
| `BITCOIN_RPC_USER` | yes | RPC user. Cannot be empty. |
| `BITCOIN_RPC_PASSWORD` | yes | RPC password. Cannot be empty. Inject from a secret manager, never bake it into an image. |
| `TANDEM_ZMQ_RAWBLOCK_URL` | no | Optional `rawblock` publisher. Only a wakeup: the payload is discarded and the block is refetched over RPC. |
| `TANDEM_POLL_INTERVAL_MS` | no | Wakeup interval. Defaults to 5000 and must be at least 250. |

The node must be out of initial block download, at matching block and header heights, on the
configured chain, and have a fully synced `txindex`. Readiness stays false otherwise, and the
historical prevout resolver needs `txindex` to reconstruct inputs.

## Service

| Variable | Required | Rule |
|---|---|---|
| `TANDEM_BIND_ADDR` | no | Listen address. Defaults to `127.0.0.1:8088`. Containers usually need `0.0.0.0:8088` for the published port to reach the process. |

## Signing

| Variable | Required | Rule |
|---|---|---|
| `TANDEM_SIGNING_KEY_FILE` | yes | Path to a file holding one 32-byte Ed25519 seed as exactly 64 hexadecimal characters. Trailing whitespace is trimmed, anything else is rejected. |
| `TANDEM_SIGNING_KEY_ID` | yes | 1 to 128 characters from `A-Z`, `a-z`, `0-9`, `.`, `_`, `:` and `-`. Consumers resolve this id through an authenticated registry. |

Restrict the seed file to the service account. Do not commit it, do not bake it into an image, and do
not reuse pipeline A's key: the whole point of the design is that the two signatures come from
separately held keys in separate trust maps.

## Release identity

| Variable | Required | Rule |
|---|---|---|
| `TANDEM_PARSER_COMMIT` | yes | 40 lowercase hexadecimal characters. |
| `TANDEM_INDEXER_COMMIT` | yes | 40 lowercase hexadecimal characters. |
| `TANDEM_PARSER_BINARY_SHA256` | yes | 64 lowercase hexadecimal characters. |
| `TANDEM_INDEXER_BINARY_SHA256` | yes | 64 lowercase hexadecimal characters. |

These four values are stored with every block as it is applied, and they are signed into every
agreement tuple. Set them from the immutable artifacts that are actually running. Because they are
persisted per height, a later release cannot relabel work done by an earlier one.

## Mainnet interlock

| Variable | Required on mainnet | Rule |
|---|---|---|
| `TANDEM_ALLOW_MAINNET` | yes | Must be `true`. Defaults to `false`, and a mainnet binding without it fails at startup. |
| `TANDEM_MAINNET_AUTHORIZATION_FILE` | yes | Path to a JSON authorization artifact. |
| `TANDEM_MAINNET_AUTHORIZATION_SHA256` | yes | SHA256 of that file, supplied separately from the file itself. |

The authorization file must parse with no unknown fields and must carry:

```json
{
  "schema": "urn:tandem:mainnet-authorization",
  "protocolId": "tndm:mainnet:<init-txid>",
  "specHash": "<64 lowercase hex>",
  "approvedBy": ["first.approver", "second.approver"],
  "approvedAt": "<non-empty timestamp>"
}
```

Its `protocolId` and `specHash` must equal the configured binding exactly, and `approvedBy` must
contain at least two distinct non-empty names. None of these variables apply on any other network.

This is an operational interlock, not authority. [`launch-gates.md`](launch-gates.md) still lists
mainnet launch authority as not granted, and every other gate on that page is still blocking.

## What is deliberately not configurable

Consensus constants are not environment driven. The carrier value, refund delay, founding window,
INIT lead and marker encoding come from the specification and live in `tandem-core`. There is no
override, and adding one would break the property the two pipelines exist to provide.
