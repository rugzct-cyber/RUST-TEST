//! Keyboard event handling for TUI
//!
//! Handles user input: quit, scroll, toggle debug logs

use std::sync::{Arc, Mutex};
use std::time::Duration;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use tokio::sync::broadcast;

use super::app::AppState;

/// Poll for keyboard events and update AppState
///
/// Returns true if should continue, false if quit requested
pub fn handle_events(
    app_state: &Arc<Mutex<AppState>>,
    shutdown_tx: &broadcast::Sender<()>,
) -> bool {
    // Non-blocking poll with 50ms timeout
    if event::poll(Duration::from_millis(50)).unwrap_or(false) {
        if let Ok(Event::Key(key)) = event::read() {
            match key.code {
                // Quit: q or Ctrl+C
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    if let Ok(mut state) = app_state.lock() {
                        state.should_quit = true;
                    }
                    let _ = shutdown_tx.send(());
                    return false;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Ok(mut state) = app_state.lock() {
                        state.should_quit = true;
                    }
                    let _ = shutdown_tx.send(());
                    return false;
                }
                
                // Scroll logs: j/k or arrows
                KeyCode::Char('j') | KeyCode::Down => {
                    if let Ok(mut state) = app_state.lock() {
                        if state.log_scroll_offset > 0 {
                            state.log_scroll_offset -= 1;
                        }
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if let Ok(mut state) = app_state.lock() {
                        state.log_scroll_offset += 1;
                    }
                }
                
                // Toggle debug logs: l
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    if let Ok(mut state) = app_state.lock() {
                        state.show_debug_logs = !state.show_debug_logs;
                    }
                }
                
                _ => {}
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_state_scroll_offset() {
        let state = Arc::new(Mutex::new(AppState::new(
            "BTC".into(), 0.1, 0.05, 0.001, 10,
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
            "BTC".into(), 0.1, 0.05, 0.001, 10,
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
}
