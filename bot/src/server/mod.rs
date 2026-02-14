//! WebSocket API server for broadcasting price data and opportunities.
//!
//! Uses `axum` for HTTP/WS routing with CORS support.

pub mod ws;

use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use axum::{
    Router,
    extract::State,
    response::Json,
    routing::get,
};
use tower_http::cors::CorsLayer;
use tracing::info;

use crate::core::aggregator::PriceAggregator;
use crate::core::types::{AggregatedPrice, BroadcastEvent};

/// Shared application state for the HTTP/WS server.
#[derive(Clone)]
pub struct AppState {
    /// Broadcast channel for real-time events → WS clients
    pub event_tx: broadcast::Sender<BroadcastEvent>,
    /// Price aggregator (for REST snapshots)
    pub aggregator: Arc<RwLock<PriceAggregator>>,
}

/// Start the HTTP/WebSocket server.
///
/// Blocks until the server shuts down.
pub async fn start_server(state: AppState, port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/prices", get(prices_handler))
        .route("/ws", get(ws::ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    info!(address = %addr, "Starting WebSocket API server");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// GET /health — server status
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "timestamp": crate::core::types::current_time_ms(),
    }))
}

/// GET /api/prices — snapshot of all aggregated prices
async fn prices_handler(
    State(state): State<AppState>,
) -> Json<Vec<AggregatedPrice>> {
    let agg = state.aggregator.read().await;
    Json(agg.get_all())
}
