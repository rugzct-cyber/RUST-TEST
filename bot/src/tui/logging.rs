//! Custom tracing Layer for TUI log capture
//!
//! Captures log events and pushes them to AppState for display.

use std::sync::{Arc, Mutex};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::app::{AppState, LogEntry};

/// Tracks whether DEBUG logs are enabled. Updated from AppState to avoid
/// acquiring the lock just to check the flag.
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Atomic counter for logs dropped due to lock contention.
/// Synced into `AppState.dropped_logs_count` when the lock is next acquired.
static DROPPED_LOGS: AtomicU64 = AtomicU64::new(0);

/// Update the global DEBUG filter flag (call from event.rs when toggling).
pub fn set_show_debug(enabled: bool) {
    SHOW_DEBUG.store(enabled, Ordering::Relaxed);
}

/// Custom Layer that captures logs for TUI display.
///
/// SAFETY: `on_event()` MUST use `try_lock()` — never `lock()` — because
/// tracing events can fire while another thread holds the AppState lock
/// (e.g. runtime logging inside a lock block). Using `lock()` here would
/// cause a deadlock. Dropped logs under contention are acceptable.
pub struct TuiLayer {
    app_state: Arc<Mutex<AppState>>,
}

impl TuiLayer {
    pub fn new(app_state: Arc<Mutex<AppState>>) -> Self {
        Self { app_state }
    }
}

impl<S: Subscriber> Layer<S> for TuiLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = event.metadata().level();

        // Filter DEBUG before acquiring lock
        if *level == tracing::Level::DEBUG && !SHOW_DEBUG.load(Ordering::Relaxed) {
            return;
        }

        // Extract message + key structured fields
        let mut message = String::new();
        let mut extra_fields = Vec::new();
        let mut visitor = MessageVisitor {
            message: &mut message,
            extra_fields: &mut extra_fields,
        };
        event.record(&mut visitor);

        // Append structured fields to message for richer TUI display
        if !extra_fields.is_empty() {
            message.push_str(" [");
            message.push_str(&extra_fields.join(", "));
            message.push(']');
        }

        let level_str = level.to_string();
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

        let entry = LogEntry {
            timestamp,
            level: level_str,
            message,
        };

        // Count dropped logs instead of silent loss
        match self.app_state.try_lock() {
            Ok(mut state) => {
                // Sync any previously dropped log count into AppState
                let dropped = DROPPED_LOGS.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    state.dropped_logs_count += dropped;
                }
                state.push_log(entry);
            }
            Err(_) => {
                // Lock contended — atomically count the drop (always succeeds)
                DROPPED_LOGS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Visitor to extract message and key structured fields from tracing events
struct MessageVisitor<'a> {
    message: &'a mut String,
    extra_fields: &'a mut Vec<String>,
}

impl<'a> tracing::field::Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.message = format!("{:?}", value).trim_matches('"').to_string();
        } else if matches!(field.name(), "event_type" | "pair" | "direction" | "error") {
            self.extra_fields
                .push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.message = value.to_string();
        } else if matches!(field.name(), "event_type" | "pair" | "direction" | "error") {
            self.extra_fields
                .push(format!("{}={}", field.name(), value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_layer_creation() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(),
            0.1,
            0.05,
            0.001,
            10,
        )));
        let layer = TuiLayer::new(Arc::clone(&state));
        // Layer created successfully - verify reference count
        assert_eq!(Arc::strong_count(&layer.app_state), 2);
    }

    #[test]
    fn test_log_entry_creation() {
        let entry = LogEntry {
            timestamp: "12:00:00".to_string(),
            level: "INFO".to_string(),
            message: "Test message".to_string(),
        };
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.message, "Test message");
    }
}
