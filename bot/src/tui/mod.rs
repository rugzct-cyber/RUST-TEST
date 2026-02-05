//! TUI Module for HFT Bot
//!
//! Optional terminal user interface activated via LOG_FORMAT=tui
//!
//! # Usage
//! ```bash
//! LOG_FORMAT=tui cargo run --release
//! ```
//!
//! # Keyboard Controls
//! - `q` or `Ctrl+C`: Quit
//! - `↑/k` `↓/j`: Scroll logs
//! - `l`: Toggle DEBUG logs

pub mod app;
pub mod event;
pub mod logging;
pub mod ui;

pub use app::{AppState, LogEntry, MAX_LOG_ENTRIES};
pub use logging::TuiLayer;
