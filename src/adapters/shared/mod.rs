//! Shared helpers for exchange adapters
//!
//! This module provides common utilities for WebSocket connection management,
//! reconnection logic, and other shared functionality across adapters.

pub mod reconnect;
pub mod websocket;

pub use reconnect::{reconnect_with_backoff, ReconnectConfig};
pub use websocket::connect_tls;
