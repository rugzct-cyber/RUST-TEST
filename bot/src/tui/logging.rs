//! Custom tracing Layer for TUI log capture
//!
//! Captures log events and pushes them to AppState for display.

use std::sync::{Arc, Mutex};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use super::app::{AppState, LogEntry};

/// Custom Layer that captures logs for TUI display
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
        // Extract message from event
        let mut message = String::new();
        let mut visitor = MessageVisitor(&mut message);
        event.record(&mut visitor);
        
        let level = event.metadata().level().to_string();
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        
        let entry = LogEntry {
            timestamp,
            level: level.clone(),
            message,
        };
        
        // Use try_lock to avoid blocking hot path
        if let Ok(mut state) = self.app_state.try_lock() {
            // Filter DEBUG if not enabled
            if level == "DEBUG" && !state.show_debug_logs {
                return;
            }
            state.push_log(entry);
        }
    }
}

/// Visitor to extract message field from tracing events
struct MessageVisitor<'a>(&'a mut String);

impl<'a> tracing::field::Visit for MessageVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{:?}", value).trim_matches('"').to_string();
        }
    }
    
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.0 = value.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_tui_layer_creation() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(), 0.1, 0.05, 0.001, 10,
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
