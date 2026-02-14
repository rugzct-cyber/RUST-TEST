//! Shared WebSocket connection helpers
//!
//! Provides TLS-enabled WebSocket connection utilities used by all adapters.

use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};

use crate::adapters::errors::ExchangeError;

/// Type alias for the WebSocket stream with TLS
pub type TlsWebSocketStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect to a WebSocket endpoint with TLS (TLSv1.2 minimum)
///
/// This helper centralizes the TLS configuration for all exchange adapters,
/// ensuring consistent security settings across connections.
///
/// # Arguments
/// * `url` - WebSocket URL to connect to (wss://)
///
/// # Returns
/// * `Ok(TlsWebSocketStream)` - Connected WebSocket stream
/// * `Err(ExchangeError)` - Connection or TLS error
pub async fn connect_tls(url: &str) -> Result<TlsWebSocketStream, ExchangeError> {
    let tls = native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .map_err(|e| ExchangeError::ConnectionFailed(format!("TLS error: {}", e)))?;

    let (ws_stream, _response) =
        connect_async_tls_with_config(url, None, false, Some(Connector::NativeTls(tls)))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

    Ok(ws_stream)
}

/// Connect with custom HTTP headers (for exchanges that require User-Agent, Origin, etc.)
pub async fn connect_tls_with_request(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
) -> Result<TlsWebSocketStream, ExchangeError> {
    let tls = native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .map_err(|e| ExchangeError::ConnectionFailed(format!("TLS error: {}", e)))?;

    let (ws_stream, _response) =
        connect_async_tls_with_config(request, None, false, Some(Connector::NativeTls(tls)))
            .await
            .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

    Ok(ws_stream)
}
