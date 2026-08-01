//! Canonical ingestion worker with atomic block commits and exact inverse reorgs.

use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tandem_core::{ChainState, ProtocolStatus, apply_block};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::config::ReleaseIdentity;
use crate::rpc::BlockSource;
use crate::store::Store;
use crate::wakeup::Wakeup;

/// Observable ingestion health without consensus authority.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkerHealth {
    /// Last successful canonical height.
    pub last_successful_height: Option<u64>,
    /// Last worker error, cleared after a successful pass.
    pub last_error: Option<String>,
    /// True while canonical catch-up is running.
    pub catching_up: bool,
}

/// Shared worker health handle.
pub type SharedWorkerHealth = Arc<RwLock<WorkerHealth>>;

/// Ingestion coordinator.
pub struct Ingestion<S: BlockSource> {
    source: Arc<S>,
    store: Store,
    state: ChainState,
    release: ReleaseIdentity,
    health: SharedWorkerHealth,
}

impl<S: BlockSource> Ingestion<S> {
    /// Construct from one explicit binding state.
    pub fn new(
        source: Arc<S>,
        store: Store,
        state: ChainState,
        release: ReleaseIdentity,
        health: SharedWorkerHealth,
    ) -> Self {
        Self {
            source,
            store,
            state,
            release,
            health,
        }
    }

    /// Run forever, retrying external failures without advancing local state.
    pub async fn run(mut self, mut wakeup: Box<dyn Wakeup>) {
        loop {
            {
                let mut health = self.health.write().await;
                health.catching_up = true;
            }
            match self.run_once().await {
                Ok(()) => {
                    let mut health = self.health.write().await;
                    health.last_successful_height = self.state.tip_height;
                    health.last_error = None;
                    health.catching_up = false;
                }
                Err(error) => {
                    error!(error = %error, "Tandem ingestion pass failed");
                    let mut health = self.health.write().await;
                    health.last_error = Some(format!("{error:#}"));
                    health.catching_up = false;
                }
            }
            if let Err(error) = wakeup.wait().await {
                warn!(error = %error, "block wakeup failed; retrying with a short delay");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }

    /// Perform one reorg, catch-up, and mempool pass.
    pub async fn run_once(&mut self) -> Result<()> {
        let status = self
            .source
            .status()
            .await
            .context("cannot read Core readiness")?;
        ensure!(
            status.ready_for(self.state.binding.network),
            "Bitcoin Core is not ready for the configured network: {status:?}"
        );
        let best_height = self.source.best_height().await?;
        self.reconcile_reorg(best_height).await?;

        let first_height = if let Some(tip) = self.state.tip_height {
            tip.checked_add(1).context("canonical height overflow")?
        } else if let Some(height) = self
            .source
            .transaction_height(self.state.binding.init_txid)
            .await?
        {
            height
        } else {
            info!(protocol_id = %self.state.binding.protocol_id(), "configured INIT is not canonical yet");
            self.store
                .replace_mempool(&self.source.mempool().await?)
                .await?;
            return Ok(());
        };

        if first_height <= best_height {
            for height in first_height..=best_height {
                let block = self.source.block(height).await.with_context(|| {
                    format!("cannot resolve canonical Bitcoin block at height {height}")
                })?;
                let delta = apply_block(&mut self.state, &block)
                    .with_context(|| format!("Tandem reduction failed at height {height}"))?;
                if let Err(error) = self
                    .store
                    .apply_block(&self.state, &delta, &self.release)
                    .await
                {
                    self.state = self.store.load_state().await.unwrap_or(delta.before);
                    return Err(error).context("PostgreSQL block transaction failed");
                }
                info!(height, block_hash = %block.hash, events = delta.events.len(), "applied Tandem block");
            }
        }

        let mempool = self
            .source
            .mempool()
            .await
            .context("cannot resolve Core mempool")?;
        self.store.replace_mempool(&mempool).await?;
        Ok(())
    }

    async fn reconcile_reorg(&mut self, best_height: u64) -> Result<()> {
        loop {
            let Some(tip_height) = self.state.tip_height else {
                return Ok(());
            };
            let matches = if tip_height > best_height {
                false
            } else {
                self.source.block_hash(tip_height).await? == self.state.tip_hash.expect("tip hash")
            };
            if matches {
                return Ok(());
            }
            let disconnected = self.state.tip_hash.expect("tip hash");
            self.state = self
                .store
                .rollback_tip()
                .await?
                .context("database has no rollback journal for current tip")?;
            warn!(height = tip_height, block_hash = %disconnected, "disconnected Tandem block after Bitcoin reorganization");
            if matches!(self.state.protocol_status, ProtocolStatus::AwaitingInit) {
                return Ok(());
            }
        }
    }
}
