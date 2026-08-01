//! Transactional `PostgreSQL` canonical store and inverse reorganization journal.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Row, Transaction};
use tandem_core::{Binding, BlockDelta, ChainState, Hash32, HeightRoots, TxView, disconnect_block};

use crate::config::ReleaseIdentity;

/// `PostgreSQL` store shared by ingestion and read APIs.
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connect, run migrations, and bind an empty database to exactly one deployment.
    pub async fn connect(database_url: &str, binding: &Binding) -> Result<(Self, ChainState)> {
        let pool = PgPoolOptions::new()
            .min_connections(1)
            .max_connections(10)
            .connect(database_url)
            .await
            .context("cannot connect to PostgreSQL")?;
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .context("cannot apply PostgreSQL migrations")?;
        let store = Self { pool };
        let state = store.bind_or_load(binding).await?;
        Ok((store, state))
    }

    /// Return a pool clone for health checks.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn bind_or_load(&self, binding: &Binding) -> Result<ChainState> {
        let mut transaction = self.pool.begin().await?;
        advisory_lock(&mut transaction).await?;
        let row = sqlx::query("SELECT binding, state FROM protocol_state WHERE id = 1 FOR UPDATE")
            .fetch_optional(&mut *transaction)
            .await?;
        let state = if let Some(row) = row {
            let stored_binding: Binding = serde_json::from_value(row.try_get("binding")?)
                .context("stored protocol binding is corrupt")?;
            ensure!(
                stored_binding == *binding,
                "database is bound to another Tandem deployment"
            );
            serde_json::from_value(row.try_get("state")?)
                .context("stored chain state is corrupt")?
        } else {
            let state = ChainState::new(binding.clone());
            sqlx::query("INSERT INTO protocol_state (id, binding, state) VALUES (1, $1, $2)")
                .bind(serde_json::to_value(binding)?)
                .bind(serde_json::to_value(&state)?)
                .execute(&mut *transaction)
                .await?;
            state
        };
        transaction.commit().await?;
        Ok(state)
    }

    /// Reload the authoritative reducer state.
    pub async fn load_state(&self) -> Result<ChainState> {
        let value: Value = sqlx::query_scalar("SELECT state FROM protocol_state WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        serde_json::from_value(value).context("stored chain state is corrupt")
    }

    /// Atomically persist one post-block state, events, roots, and inverse journal.
    pub async fn apply_block(
        &self,
        post_state: &ChainState,
        delta: &BlockDelta,
        release: &ReleaseIdentity,
    ) -> Result<()> {
        ensure!(
            post_state.tip_height == Some(delta.height),
            "post-state height does not match delta"
        );
        ensure!(
            post_state.tip_hash == Some(delta.block_hash),
            "post-state hash does not match delta"
        );
        let mut transaction = self.pool.begin().await?;
        advisory_lock(&mut transaction).await?;
        let stored = load_state_for_update(&mut transaction).await?;
        ensure!(
            stored == delta.before,
            "database state changed before block commit"
        );

        sqlx::query(
            "INSERT INTO canonical_blocks (
                height, block_hash, previous_hash, event_root, object_state_root, chained_root,
                founding_created, all_objects, active_objects, parser_commit, indexer_commit,
                parser_binary_sha256, indexer_binary_sha256, inverse_delta
             ) VALUES ($1,$2,$3,$4,$5,$6,$7::numeric,$8::numeric,$9::numeric,$10,$11,$12,$13,$14)",
        )
        .bind(to_i64(delta.height)?)
        .bind(delta.block_hash.0.to_vec())
        .bind(delta.previous_hash.0.to_vec())
        .bind(delta.roots.event_root.0.to_vec())
        .bind(delta.roots.object_state_root.0.to_vec())
        .bind(delta.roots.chained_root.0.to_vec())
        .bind(delta.roots.counters.founding_created.to_string())
        .bind(delta.roots.counters.all_objects.to_string())
        .bind(delta.roots.counters.active_objects.to_string())
        .bind(&release.parser_commit)
        .bind(&release.indexer_commit)
        .bind(release.parser_binary_sha256.0.to_vec())
        .bind(release.indexer_binary_sha256.0.to_vec())
        .bind(serde_json::to_value(delta)?)
        .execute(&mut *transaction)
        .await
        .context("cannot insert canonical block")?;

        for event in &delta.events {
            sqlx::query(
                "INSERT INTO canonical_events (
                    height, tx_index, event_index, sub_index, txid, object_key,
                    event_type, validity_class, reason, payload
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            )
            .bind(to_i64(event.height)?)
            .bind(i64::from(event.tx_index))
            .bind(i64::from(event.event_index))
            .bind(i64::from(event.sub_index))
            .bind(event.txid.0.to_vec())
            .bind(event.object_key.0.to_vec())
            .bind(i16::from(event.event_type as u8))
            .bind(i16::from(event.validity_class as u8))
            .bind(i32::from(event.reason as u16))
            .bind(serde_json::to_value(event)?)
            .execute(&mut *transaction)
            .await?;
        }
        sync_objects(&mut transaction, post_state).await?;
        save_state(&mut transaction, post_state).await?;
        transaction
            .commit()
            .await
            .context("cannot commit canonical block")?;
        Ok(())
    }

    /// Roll back exactly one current canonical tip and journal the reorganization.
    pub async fn rollback_tip(&self) -> Result<Option<ChainState>> {
        let mut transaction = self.pool.begin().await?;
        advisory_lock(&mut transaction).await?;
        let mut state = load_state_for_update(&mut transaction).await?;
        let Some(tip_height) = state.tip_height else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let row = sqlx::query(
            "SELECT block_hash, inverse_delta FROM canonical_blocks WHERE height = $1 FOR UPDATE",
        )
        .bind(to_i64(tip_height)?)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            bail!("canonical tip has no inverse journal");
        };
        let delta: BlockDelta = serde_json::from_value(row.try_get("inverse_delta")?)
            .context("inverse block journal is corrupt")?;
        disconnect_block(&mut state, &delta).context("inverse block journal does not match tip")?;
        sqlx::query("DELETE FROM canonical_blocks WHERE height = $1")
            .bind(to_i64(tip_height)?)
            .execute(&mut *transaction)
            .await?;
        sync_objects(&mut transaction, &state).await?;
        save_state(&mut transaction, &state).await?;
        sqlx::query(
            "INSERT INTO reorg_journal (
                disconnected_height, disconnected_hash, restored_height, restored_hash
             ) VALUES ($1,$2,$3,$4)",
        )
        .bind(to_i64(tip_height)?)
        .bind(delta.block_hash.0.to_vec())
        .bind(state.tip_height.map(to_i64).transpose()?)
        .bind(state.tip_hash.map(|hash| hash.0.to_vec()))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Some(state))
    }

    /// Replace the provisional mempool overlay in one transaction.
    pub async fn replace_mempool(&self, transactions: &[TxView]) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM mempool_overlay")
            .execute(&mut *transaction)
            .await?;
        for transaction_view in transactions {
            sqlx::query("INSERT INTO mempool_overlay (txid, payload) VALUES ($1,$2)")
                .bind(transaction_view.txid.0.to_vec())
                .bind(serde_json::to_value(transaction_view)?)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Read a canonical height root tuple.
    pub async fn roots_at(&self, height: u64) -> Result<Option<PersistedHeight>> {
        let row = sqlx::query(
            "SELECT block_hash, event_root, object_state_root, chained_root,
                    founding_created::text, all_objects::text, active_objects::text,
                    parser_commit, indexer_commit, parser_binary_sha256, indexer_binary_sha256
             FROM canonical_blocks WHERE height = $1",
        )
        .bind(to_i64(height)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| roots_from_row(height, &row)).transpose()
    }

    /// Read one object by binary object key.
    pub async fn object(&self, key: Hash32) -> Result<Option<Value>> {
        sqlx::query_scalar("SELECT payload FROM canonical_objects WHERE object_key = $1")
            .bind(key.0.to_vec())
            .fetch_optional(&self.pool)
            .await
            .context("object query failed")
    }

    /// Read one active carrier by exact wire-order outpoint.
    pub async fn carrier(&self, txid: Hash32, vout: u32) -> Result<Option<Value>> {
        sqlx::query_scalar(
            "SELECT payload FROM canonical_objects
             WHERE current_txid = $1 AND current_vout = $2 AND status = 0",
        )
        .bind(txid.0.to_vec())
        .bind(i64::from(vout))
        .fetch_optional(&self.pool)
        .await
        .context("carrier query failed")
    }

    /// Read bounded events with optional height and object filters.
    pub async fn events(
        &self,
        height: Option<u64>,
        object_key: Option<Hash32>,
    ) -> Result<Vec<Value>> {
        let height = height.map(to_i64).transpose()?;
        sqlx::query_scalar(
            "SELECT payload FROM canonical_events
             WHERE ($1::bigint IS NULL OR height = $1)
               AND ($2::bytea IS NULL OR object_key = $2)
             ORDER BY height DESC, tx_index, event_index, sub_index
             LIMIT 500",
        )
        .bind(height)
        .bind(object_key.map(|key| key.0.to_vec()))
        .fetch_all(&self.pool)
        .await
        .context("event query failed")
    }

    /// Read bounded invalid and noncanonical events.
    pub async fn invalid_events(&self) -> Result<Vec<Value>> {
        sqlx::query_scalar(
            "SELECT payload FROM canonical_events
             WHERE validity_class <> 1
             ORDER BY height DESC, tx_index, event_index, sub_index
             LIMIT 500",
        )
        .fetch_all(&self.pool)
        .await
        .context("invalid event query failed")
    }

    /// Read recent reorganization records.
    pub async fn reorgs(&self) -> Result<Vec<ReorgRecord>> {
        let rows = sqlx::query(
            "SELECT id, disconnected_height, disconnected_hash,
                    restored_height, restored_hash, observed_at::text
             FROM reorg_journal ORDER BY id DESC LIMIT 200",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ReorgRecord {
                    id: row.try_get("id")?,
                    disconnected_height: row.try_get("disconnected_height")?,
                    disconnected_hash: hash_from_bytes(row.try_get("disconnected_hash")?)?,
                    restored_height: row.try_get("restored_height")?,
                    restored_hash: row
                        .try_get::<Option<Vec<u8>>, _>("restored_hash")?
                        .map(hash_from_bytes)
                        .transpose()?,
                    observed_at: row.try_get("observed_at")?,
                })
            })
            .collect()
    }

    /// Read current canonical state and basic database counts.
    pub async fn stats(&self) -> Result<Stats> {
        let state = self.load_state().await?;
        let invalid_events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM canonical_events WHERE validity_class <> 1")
                .fetch_one(&self.pool)
                .await?;
        let mempool_transactions: i64 = sqlx::query_scalar("SELECT count(*) FROM mempool_overlay")
            .fetch_one(&self.pool)
            .await?;
        Ok(Stats {
            protocol_id: state.binding.protocol_id(),
            protocol_status: serde_json::to_value(&state.protocol_status)?,
            tip_height: state.tip_height,
            tip_hash: state.tip_hash,
            chained_root: state.chained_root,
            counters: state.counters(),
            invalid_events,
            mempool_transactions,
        })
    }

    /// Read the provisional mempool overlay.
    pub async fn mempool(&self) -> Result<Vec<Value>> {
        sqlx::query_scalar(
            "SELECT payload FROM mempool_overlay ORDER BY observed_at DESC LIMIT 1000",
        )
        .fetch_all(&self.pool)
        .await
        .context("mempool query failed")
    }
}

async fn advisory_lock(transaction: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(607841294141560657)")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn load_state_for_update(transaction: &mut Transaction<'_, Postgres>) -> Result<ChainState> {
    let value: Value =
        sqlx::query_scalar("SELECT state FROM protocol_state WHERE id = 1 FOR UPDATE")
            .fetch_one(&mut **transaction)
            .await?;
    serde_json::from_value(value).context("stored chain state is corrupt")
}

async fn save_state(transaction: &mut Transaction<'_, Postgres>, state: &ChainState) -> Result<()> {
    sqlx::query("UPDATE protocol_state SET state = $1, updated_at = now() WHERE id = 1")
        .bind(serde_json::to_value(state)?)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn sync_objects(
    transaction: &mut Transaction<'_, Postgres>,
    state: &ChainState,
) -> Result<()> {
    sqlx::query("DELETE FROM canonical_objects")
        .execute(&mut **transaction)
        .await?;
    for object in state.objects.values() {
        sqlx::query(
            "INSERT INTO canonical_objects (
                object_key, current_txid, current_vout, status, create_height, state_seq, payload
             ) VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(object.object_key.0.to_vec())
        .bind(object.current_outpoint.txid.0.to_vec())
        .bind(i64::from(object.current_outpoint.vout))
        .bind(i16::from(object.status as u8))
        .bind(to_i64(object.create_height)?)
        .bind(i64::from(object.state_seq))
        .bind(serde_json::to_value(object)?)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn roots_from_row(height: u64, row: &sqlx::postgres::PgRow) -> Result<PersistedHeight> {
    Ok(PersistedHeight {
        roots: HeightRoots {
            height,
            block_hash: hash_from_bytes(row.try_get("block_hash")?)?,
            event_root: hash_from_bytes(row.try_get("event_root")?)?,
            object_state_root: hash_from_bytes(row.try_get("object_state_root")?)?,
            chained_root: hash_from_bytes(row.try_get("chained_root")?)?,
            counters: tandem_core::Counters {
                founding_created: row
                    .try_get::<String, _>("founding_created")?
                    .parse()
                    .context("invalid founding counter")?,
                all_objects: row
                    .try_get::<String, _>("all_objects")?
                    .parse()
                    .context("invalid object counter")?,
                active_objects: row
                    .try_get::<String, _>("active_objects")?
                    .parse()
                    .context("invalid active counter")?,
            },
        },
        release: ReleaseIdentity {
            parser_commit: row.try_get("parser_commit")?,
            indexer_commit: row.try_get("indexer_commit")?,
            parser_binary_sha256: hash_from_bytes(row.try_get("parser_binary_sha256")?)?,
            indexer_binary_sha256: hash_from_bytes(row.try_get("indexer_binary_sha256")?)?,
        },
    })
}

fn hash_from_bytes(bytes: Vec<u8>) -> Result<Hash32> {
    Ok(Hash32(bytes.try_into().map_err(|_| {
        anyhow::anyhow!("database hash is not 32 bytes")
    })?))
}

fn to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("height exceeds PostgreSQL bigint")
}

/// Canonical roots plus the exact release that produced them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersistedHeight {
    /// Per-height consensus roots and counters.
    pub roots: HeightRoots,
    /// Source and artifact identity used for block processing.
    pub release: ReleaseIdentity,
}

/// Public reorganization journal row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReorgRecord {
    /// Monotonic database identifier.
    pub id: i64,
    /// Disconnected height.
    pub disconnected_height: i64,
    /// Disconnected block hash in wire order.
    pub disconnected_hash: Hash32,
    /// Restored tip height.
    pub restored_height: Option<i64>,
    /// Restored tip hash in wire order.
    pub restored_hash: Option<Hash32>,
    /// `PostgreSQL` observation time.
    pub observed_at: String,
}

/// Public service statistics.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stats {
    /// Bound protocol identifier.
    pub protocol_id: String,
    /// Current protocol lifecycle.
    pub protocol_status: Value,
    /// Canonical tip height.
    pub tip_height: Option<u64>,
    /// Canonical tip hash.
    pub tip_hash: Option<Hash32>,
    /// Current chained root.
    pub chained_root: Hash32,
    /// Canonical counters.
    pub counters: tandem_core::Counters,
    /// Number of retained invalid events.
    pub invalid_events: i64,
    /// Number of current provisional mempool transactions.
    pub mempool_transactions: i64,
}
