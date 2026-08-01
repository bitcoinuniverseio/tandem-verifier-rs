CREATE TABLE protocol_state (
    id smallint PRIMARY KEY CHECK (id = 1),
    binding jsonb NOT NULL,
    state jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE canonical_blocks (
    height bigint PRIMARY KEY CHECK (height >= 0),
    block_hash bytea NOT NULL UNIQUE CHECK (octet_length(block_hash) = 32),
    previous_hash bytea NOT NULL CHECK (octet_length(previous_hash) = 32),
    event_root bytea NOT NULL CHECK (octet_length(event_root) = 32),
    object_state_root bytea NOT NULL CHECK (octet_length(object_state_root) = 32),
    chained_root bytea NOT NULL CHECK (octet_length(chained_root) = 32),
    founding_created numeric(20,0) NOT NULL CHECK (founding_created >= 0),
    all_objects numeric(20,0) NOT NULL CHECK (all_objects >= 0),
    active_objects numeric(20,0) NOT NULL CHECK (active_objects >= 0),
    parser_commit text NOT NULL CHECK (parser_commit ~ '^[0-9a-f]{40}$'),
    indexer_commit text NOT NULL CHECK (indexer_commit ~ '^[0-9a-f]{40}$'),
    parser_binary_sha256 bytea NOT NULL CHECK (octet_length(parser_binary_sha256) = 32),
    indexer_binary_sha256 bytea NOT NULL CHECK (octet_length(indexer_binary_sha256) = 32),
    inverse_delta jsonb NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE canonical_events (
    height bigint NOT NULL REFERENCES canonical_blocks(height) ON DELETE CASCADE,
    tx_index bigint NOT NULL,
    event_index bigint NOT NULL,
    sub_index bigint NOT NULL,
    txid bytea NOT NULL CHECK (octet_length(txid) = 32),
    object_key bytea NOT NULL CHECK (octet_length(object_key) = 32),
    event_type smallint NOT NULL,
    validity_class smallint NOT NULL,
    reason integer NOT NULL,
    payload jsonb NOT NULL,
    PRIMARY KEY (height, tx_index, event_index, sub_index)
);

CREATE INDEX canonical_events_object_key_idx ON canonical_events(object_key, height, tx_index);
CREATE INDEX canonical_events_reason_idx ON canonical_events(reason, height, tx_index);
CREATE INDEX canonical_events_txid_idx ON canonical_events(txid);

CREATE TABLE canonical_objects (
    object_key bytea PRIMARY KEY CHECK (octet_length(object_key) = 32),
    current_txid bytea NOT NULL CHECK (octet_length(current_txid) = 32),
    current_vout bigint NOT NULL,
    status smallint NOT NULL,
    create_height bigint NOT NULL,
    state_seq bigint NOT NULL,
    payload jsonb NOT NULL
);

CREATE INDEX canonical_objects_current_outpoint_idx ON canonical_objects(current_txid, current_vout);
CREATE INDEX canonical_objects_status_idx ON canonical_objects(status, create_height);

CREATE TABLE mempool_overlay (
    txid bytea PRIMARY KEY CHECK (octet_length(txid) = 32),
    payload jsonb NOT NULL,
    observed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE reorg_journal (
    id bigserial PRIMARY KEY,
    disconnected_height bigint NOT NULL,
    disconnected_hash bytea NOT NULL CHECK (octet_length(disconnected_hash) = 32),
    restored_height bigint,
    restored_hash bytea CHECK (restored_hash IS NULL OR octet_length(restored_hash) = 32),
    observed_at timestamptz NOT NULL DEFAULT now()
);
