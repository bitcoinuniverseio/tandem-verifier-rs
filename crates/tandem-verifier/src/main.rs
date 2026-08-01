//! Tandem verifier service executable.

use std::sync::Arc;

use anyhow::Result;
use tandem_verifier::api::{ApiState, router};
use tandem_verifier::config::Config;
use tandem_verifier::ingest::{Ingestion, SharedWorkerHealth, WorkerHealth};
use tandem_verifier::rpc::BitcoinCoreRpc;
use tandem_verifier::signer::AgreementSigner;
use tandem_verifier::store::Store;
use tandem_verifier::wakeup::{PollWakeup, Wakeup};
use tokio::sync::RwLock;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(Config::load()?);
    let signer = Arc::new(AgreementSigner::load(
        &config.signing_key_file,
        config.signing_key_id.clone(),
    )?);
    let (store, chain_state) = Store::connect(&config.database_url, &config.binding).await?;
    let source = Arc::new(BitcoinCoreRpc::new(&config)?);
    let worker_health: SharedWorkerHealth = Arc::new(RwLock::new(WorkerHealth::default()));
    let wakeup = build_wakeup(&config).await?;

    let ingestion = Ingestion::new(
        Arc::clone(&source),
        store.clone(),
        chain_state,
        config.release_identity(),
        Arc::clone(&worker_health),
    );
    tokio::spawn(ingestion.run(wakeup));

    let app = router(ApiState {
        config: Arc::clone(&config),
        store,
        source,
        signer,
        worker_health,
    });
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(address = %config.bind_addr, protocol_id = %config.binding.protocol_id(), "Tandem verifier listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn build_wakeup(config: &Config) -> Result<Box<dyn Wakeup>> {
    if let Some(endpoint) = &config.zmq_rawblock_url {
        #[cfg(feature = "zmq")]
        {
            return Ok(Box::new(
                tandem_verifier::wakeup::ZmqWakeup::connect(endpoint).await?,
            ));
        }
        #[cfg(not(feature = "zmq"))]
        {
            let _ = endpoint;
            anyhow::bail!("TANDEM_ZMQ_RAWBLOCK_URL is set but this binary lacks the zmq feature");
        }
    }
    Ok(Box::new(PollWakeup::new(config.poll_interval)))
}

async fn shutdown_signal() {
    let control_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}
