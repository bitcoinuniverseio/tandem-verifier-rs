//! Read-only verifier HTTP API and fail-closed readiness response.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Executor as _;
use tandem_core::{Hash32, ProtocolStatus, wire_hash};

use crate::config::Config;
use crate::ingest::SharedWorkerHealth;
use crate::rpc::{BitcoinCoreRpc, BlockSource, CoreStatus};
use crate::signer::{AgreementSigner, AgreementTuple};
use crate::store::Store;

/// Shared API dependencies.
#[derive(Clone)]
pub struct ApiState {
    /// Validated process configuration.
    pub config: Arc<Config>,
    /// Canonical `PostgreSQL` store.
    pub store: Store,
    /// Bitcoin Core boundary.
    pub source: Arc<BitcoinCoreRpc>,
    /// Agreement signer.
    pub signer: Arc<AgreementSigner>,
    /// Ingestion health.
    pub worker_health: SharedWorkerHealth,
}

/// Build the complete read-only router.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/v1/objects/{object_key}", get(object))
        .route("/v1/carriers/{txid}/{vout}", get(carrier))
        .route("/v1/events", get(events))
        .route("/v1/invalid", get(invalid_events))
        .route("/v1/reorgs", get(reorgs))
        .route("/v1/stats", get(stats))
        .route("/v1/mempool", get(mempool))
        .route("/v1/agreement/{height}", get(agreement))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({"status": "alive"}))
}

async fn readiness(State(state): State<ApiState>) -> Response {
    let mut failures = Vec::new();
    if let Err(error) = state.store.pool().execute("SELECT 1").await {
        tracing::error!(error = %error, "readiness PostgreSQL query failed");
        failures.push("postgresql: unavailable".to_owned());
    }
    let core = match state.source.status().await {
        Ok(status) => {
            if !status.ready_for(state.config.binding.network) {
                failures
                    .push("bitcoin_core: network, sync, IBD, or txindex gate failed".to_owned());
            }
            Some(status)
        }
        Err(error) => {
            tracing::error!(error = %error, "readiness Bitcoin Core query failed");
            failures.push("bitcoin_core: unavailable".to_owned());
            None
        }
    };
    let chain_state = match state.store.load_state().await {
        Ok(chain_state) => {
            match chain_state.protocol_status {
                ProtocolStatus::AwaitingInit => {
                    failures.push("protocol: configured INIT is not canonical".to_owned());
                }
                ProtocolStatus::FailedInit { reason, .. } => {
                    failures.push(format!("protocol: configured INIT failed with {reason:?}"));
                }
                ProtocolStatus::Active { .. } => {}
            }
            Some(chain_state)
        }
        Err(error) => {
            tracing::error!(error = %error, "readiness chain state query failed");
            failures.push("state: unavailable".to_owned());
            None
        }
    };
    let worker = state.worker_health.read().await.clone();
    if worker.catching_up {
        failures.push("worker: canonical catch-up is in progress".to_owned());
    }
    if worker.last_error.is_some() {
        failures.push("worker: ingestion pass failed".to_owned());
    }
    if let (Some(core_status), Some(stored_state)) = (&core, &chain_state)
        && stored_state.tip_height != Some(core_status.blocks)
    {
        failures.push("protocol: canonical tip is behind Bitcoin Core".to_owned());
    }
    let ready = failures.is_empty();
    let response = Readiness {
        ready,
        protocol_id: state.config.binding.protocol_id(),
        tip_height: chain_state.as_ref().and_then(|value| value.tip_height),
        core,
        signer_key_id: state.config.signing_key_id.clone(),
        signer_public_key: state.signer.public_key_hex(),
        worker,
        failures,
    };
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(response),
    )
        .into_response()
}

async fn object(
    State(state): State<ApiState>,
    Path(object_key): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let key = parse_hash(&object_key, "object key")?;
    state
        .store
        .object(key)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("object not found"))
}

async fn carrier(
    State(state): State<ApiState>,
    Path((txid, vout)): Path<(String, u32)>,
) -> Result<Json<Value>, ApiError> {
    let txid = wire_hash(&txid).map_err(|_| ApiError::bad_request("invalid display-order txid"))?;
    state
        .store
        .carrier(txid, vout)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("active carrier not found"))
}

#[derive(Deserialize)]
struct EventQuery {
    height: Option<u64>,
    object_key: Option<String>,
}

async fn events(
    State(state): State<ApiState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let object_key = query
        .object_key
        .as_deref()
        .map(|value| parse_hash(value, "object key"))
        .transpose()?;
    Ok(Json(state.store.events(query.height, object_key).await?))
}

async fn invalid_events(State(state): State<ApiState>) -> Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(state.store.invalid_events().await?))
}

async fn reorgs(State(state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.store.reorgs().await?)?))
}

async fn stats(State(state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(state.store.stats().await?)?))
}

async fn mempool(State(state): State<ApiState>) -> Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(state.store.mempool().await?))
}

async fn agreement(
    State(state): State<ApiState>,
    Path(height): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    let worker = state.worker_health.read().await;
    if worker.catching_up || worker.last_error.is_some() {
        return Err(ApiError::unavailable("agreement signer is not caught up"));
    }
    drop(worker);
    let core_status = state.source.status().await?;
    if !core_status.ready_for(state.config.binding.network) {
        return Err(ApiError::unavailable("Bitcoin Core readiness gate failed"));
    }
    let chain_state = state.store.load_state().await?;
    if !matches!(chain_state.protocol_status, ProtocolStatus::Active { .. })
        || chain_state.tip_height != Some(core_status.blocks)
        || height > core_status.blocks
    {
        return Err(ApiError::unavailable(
            "canonical protocol state is not ready for signing",
        ));
    }
    let persisted = state
        .store
        .roots_at(height)
        .await?
        .ok_or_else(|| ApiError::not_found("canonical height not found"))?;
    let canonical_hash = state.source.block_hash(height).await?;
    if canonical_hash != persisted.roots.block_hash {
        return Err(ApiError::conflict(
            "stored height is not canonical on the configured Bitcoin Core node",
        ));
    }
    let tuple = AgreementTuple::from_roots(
        &state.config.binding.protocol_id(),
        &persisted.roots,
        &persisted.release,
    );
    Ok(Json(serde_json::to_value(state.signer.sign(tuple)?)?))
}

fn parse_hash(value: &str, label: &str) -> Result<Hash32, ApiError> {
    Hash32::from_hex(value).map_err(|_| ApiError::bad_request(format!("invalid {label}")))
}

#[derive(Serialize)]
struct Readiness {
    ready: bool,
    protocol_id: String,
    tip_height: Option<u64>,
    core: Option<CoreStatus>,
    signer_key_id: String,
    signer_public_key: String,
    worker: crate::ingest::WorkerHealth,
    failures: Vec<String>,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        let error = error.into();
        tracing::error!(error = %error, "API query failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal verifier error".to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}
