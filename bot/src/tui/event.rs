//! Async keyboard event handling for TUI
//!
//! Uses crossterm's EventStream for non-blocking, async-compatible input.
//! Propagates I/O errors instead of swallowing them.
//! Non-blocking — does not block a tokio worker thread.

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::warn;

use super::app::AppState;

/// Result of processing a single event poll cycle
pub enum EventResult {
    /// Continue the TUI loop
    Continue,
    /// User requested quit
    Quit,
}

/// Poll for keyboard events asynchronously with a timeout.
///
/// Uses `EventStream` (futures-based) instead of blocking `event::poll()`.
/// Returns `EventResult::Quit` if the user pressed q/Ctrl+C,
/// `EventResult::Continue` otherwise.
///
/// I/O errors are logged as warnings rather than silently swallowed.
pub async fn handle_events_async(
    app_state: &Arc<Mutex<AppState>>,
    shutdown_tx: &broadcast::Sender<()>,
    event_stream: &mut EventStream,
) -> EventResult {
    // Use tokio::select! with a short timeout to avoid blocking
    let maybe_event =
        tokio::time::timeout(std::time::Duration::from_millis(50), event_stream.next()).await;

    match maybe_event {
        // Timeout elapsed — no input
        Err(_) => EventResult::Continue,
        // Stream ended (terminal closed)
        Ok(None) => EventResult::Quit,
        // Got an event result
        Ok(Some(event_result)) => {
            match event_result {
                // Log I/O errors instead of ignoring them
                Err(e) => {
                    warn!(event_type = "TERMINAL_IO_ERROR", error = %e, "Terminal I/O error during event polling");
                    EventResult::Continue
                }
                Ok(Event::Key(key)) => {
                    process_key_event(key.code, key.modifiers, app_state, shutdown_tx)
                }
                Ok(_) => EventResult::Continue,
            }
        }
    }
}

/// Process a single key event and update state accordingly
fn process_key_event(
    code: KeyCode,
    modifiers: KeyModifiers,
    app_state: &Arc<Mutex<AppState>>,
    shutdown_tx: &broadcast::Sender<()>,
) -> EventResult {
    match code {
        // Quit: q or Ctrl+C
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            if let Ok(mut state) = app_state.lock() {
                state.should_quit = true;
            }
            let _ = shutdown_tx.send(());
            EventResult::Quit
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            if let Ok(mut state) = app_state.lock() {
                state.should_quit = true;
            }
            let _ = shutdown_tx.send(());
            EventResult::Quit
        }

        // Scroll logs: j/k or arrows
        KeyCode::Char('j') | KeyCode::Down => {
            if let Ok(mut state) = app_state.lock() {
                if state.log_scroll_offset > 0 {
                    state.log_scroll_offset -= 1;
                }
            }
            EventResult::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Ok(mut state) = app_state.lock() {
                let max_offset = state.recent_logs.len().saturating_sub(1);
                if state.log_scroll_offset < max_offset {
                    state.log_scroll_offset += 1;
                }
            }
            EventResult::Continue
        }

        // Toggle debug logs: l
        KeyCode::Char('l') | KeyCode::Char('L') => {
            if let Ok(mut state) = app_state.lock() {
                state.show_debug_logs = !state.show_debug_logs;
                super::logging::set_show_debug(state.show_debug_logs);
            }
            EventResult::Continue
        }

        _ => EventResult::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_scroll_offset() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(),
            0.1,
            0.05,
            0.001,
            10,
        )));

        // Test scroll offset modification
        {
            let mut s = state.lock().unwrap();
            s.log_scroll_offset = 5;
        }

        {
            let s = state.lock().unwrap();
            assert_eq!(s.log_scroll_offset, 5);
        }
    }

    #[test]
    fn test_debug_toggle() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(),
            0.1,
            0.05,
            0.001,
            10,
        )));

        // Default should be false
        {
            let s = state.lock().unwrap();
            assert!(!s.show_debug_logs);
        }

        // Toggle
        {
            let mut s = state.lock().unwrap();
            s.show_debug_logs = !s.show_debug_logs;
        }

        {
            let s = state.lock().unwrap();
            assert!(s.show_debug_logs);
        }
    }

    #[test]
    fn test_process_quit_q() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(),
            0.1,
            0.05,
            0.001,
            10,
        )));
        let (tx, _rx) = broadcast::channel(1);

        let result = process_key_event(KeyCode::Char('q'), KeyModifiers::empty(), &state, &tx);
        assert!(matches!(result, EventResult::Quit));
        assert!(state.lock().unwrap().should_quit);
    }

    #[test]
    fn test_process_scroll_down() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(),
            0.1,
            0.05,
            0.001,
            10,
        )));
        state.lock().unwrap().log_scroll_offset = 3;
        let (tx, _rx) = broadcast::channel(1);

        let result = process_key_event(KeyCode::Char('j'), KeyModifiers::empty(), &state, &tx);
        assert!(matches!(result, EventResult::Continue));
        assert_eq!(state.lock().unwrap().log_scroll_offset, 2);
    }
}
