//! WebSocket handler for real-time event streaming.
//!
//! Clients connect to `/ws` and receive JSON events:
//! - `{ "type": "price", "data": { ... } }`
//! - `{ "type": "opportunity", "data": { ... } }`

use axum::{
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::IntoResponse,
};
use tracing::{info, warn};

use super::AppState;

/// WebSocket upgrade handler at GET /ws
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Handle an individual WebSocket connection.
///
/// Subscribes to the broadcast channel and forwards all events as JSON.
async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let mut rx = state.event_tx.subscribe();
    let peer = "ws-client"; // axum doesn't expose peer addr easily

    info!(peer = peer, "WebSocket client connected");

    loop {
        tokio::select! {
            // Forward broadcast events to the WS client
            event = rx.recv() => {
                match event {
                    Ok(evt) => {
                        match serde_json::to_string(&evt) {
                            Ok(json) => {
                                if socket.send(Message::Text(json.into())).await.is_err() {
                                    // Client disconnected
                                    break;
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to serialize event");
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, peer = peer, "WS client lagged, skipped events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            // Handle incoming messages from client (ping/pong + close)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {
                        // Ignore other incoming messages (text, binary)
                    }
                }
            }
        }
    }

    info!(peer = peer, "WebSocket client disconnected");
}
