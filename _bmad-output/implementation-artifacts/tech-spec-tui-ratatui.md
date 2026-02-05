---
title: 'Ratatui TUI for HFT Bot'
slug: 'tui-ratatui'
created: '2026-02-05'
status: 'in-progress'
stepsCompleted: [1, 2]
tech_stack: ['ratatui 0.29', 'crossterm 0.28']
files_to_modify:
  - 'bot/Cargo.toml'
  - 'bot/src/lib.rs'
  - 'bot/src/main.rs'
  - 'bot/src/config/logging.rs'
  - 'bot/src/core/runtime.rs'
  - 'bot/src/core/execution.rs'
files_to_create:
  - 'bot/src/tui/mod.rs'
  - 'bot/src/tui/app.rs'
  - 'bot/src/tui/ui.rs'
  - 'bot/src/tui/event.rs'
  - 'bot/src/tui/logging.rs'
code_patterns: ['Arc<Mutex<AppState>>', 'tracing Layer', 'crossterm raw mode']
test_patterns: ['unit tests for AppState', 'TuiLayer capture tests']
---

# Tech-Spec: Ratatui TUI for HFT Bot

**Created:** 2026-02-05

## Overview

### Problem Statement

Le bot HFT fonctionne uniquement via logs JSON/pretty sur stdout. L'opérateur ne peut pas visualiser en temps réel :
- Les prix bid/ask des deux exchanges
- Le spread actuel et son évolution
- L'état des positions
- Les statistiques de trading

### Solution

Ajouter une TUI (Terminal User Interface) optionnelle utilisant **ratatui** qui affiche en temps réel les données du bot dans une interface terminal structurée, tout en préservant le mode headless existant.

### Scope

**In Scope:**
- Module `tui/` avec 5 fichiers (mod, app, ui, event, logging)
- `AppState` partagé via `Arc<Mutex<>>` entre les tasks
- Layout 4 zones : header, orderbooks, stats, logs
- `TuiLayer` custom pour capturer les logs tracing
- Mode `LOG_FORMAT=tui` optionnel (opt-in)
- Panic hook pour restore terminal
- Tests unitaires pour AppState et TuiLayer

**Out of Scope:**
- Onglets multiples
- Graphiques/charts
- Persistance de configuration TUI
- Thèmes configurables

## Context for Development

### Codebase Patterns

| Pattern | Location | Description |
|---------|----------|-------------|
| Shared state | `SharedOrderbooks` | `Arc<RwLock<HashMap>>` pour partage lock-free |
| Logging init | `config/logging.rs` | `init_logging()` avec JSON/pretty modes |
| Event system | `core/events.rs` | `TradingEvent` enum avec 13+ types |
| Shutdown | `main.rs:152` | `broadcast::channel<()>` pour signal |
| Tasks tokio | `main.rs:186-221` | Pattern spawn avec shutdown receiver |

### Files to Reference

| File | Purpose |
|------|---------|
| `bot/src/main.rs` | Point d'intégration TUI (L32-271) |
| `bot/src/config/logging.rs` | Extension pour mode TUI (L24-42) |
| `bot/src/core/monitoring.rs` | Source des données orderbook |
| `bot/src/core/events.rs` | TradingEvent types pour capture |
| `bot/Cargo.toml` | Ajout dépendances ratatui/crossterm |

### Technical Decisions

1. **`Arc<Mutex<AppState>>`** plutôt que `RwLock` : Le TUI écrit rarement (100ms), les locks seront courts
2. **`try_lock()`** dans hot path : Évite de bloquer monitoring_task (25ms polling)
3. **100ms tick rate TUI** : Suffisant pour l'affichage, ne gêne pas le HFT
4. **Ring buffer 100 logs** : `VecDeque` avec rotation automatique
5. **Panic hook** : Restore terminal même en cas de crash

---

## Implementation Plan

### Task 1: Ajouter dépendances TUI (Cargo.toml)

**File:** `bot/Cargo.toml`

**Action:** Ajouter après ligne 62 (après `rand = "0.8"`):
```toml
# TUI (opt-in via LOG_FORMAT=tui)
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
```

---

### Task 2: Créer AppState (bot/src/tui/app.rs)

**File:** `bot/src/tui/app.rs` [NEW]

**Content:**
```rust
//! TUI Application State
//!
//! Shared state container for real-time display data.
//! Wrapped in Arc<Mutex<>> for safe sharing between tasks.

use std::collections::VecDeque;
use std::time::Instant;
use crate::core::spread::SpreadDirection;

/// Maximum number of log entries to keep in memory
pub const MAX_LOG_ENTRIES: usize = 100;

/// Single log entry for display
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

/// Central application state shared between TUI and bot tasks
#[derive(Debug)]
pub struct AppState {
    // Orderbooks live
    pub vest_best_bid: f64,
    pub vest_best_ask: f64,
    pub paradex_best_bid: f64,
    pub paradex_best_ask: f64,
    
    // Spread actuel
    pub current_spread_pct: f64,
    pub spread_direction: Option<SpreadDirection>,
    
    // Position
    pub position_open: bool,
    pub entry_spread: Option<f64>,
    pub entry_direction: Option<SpreadDirection>,
    pub position_polls: u64,
    
    // Config
    pub pair: String,
    pub spread_entry_threshold: f64,
    pub spread_exit_threshold: f64,
    pub position_size: f64,
    pub leverage: u32,
    
    // Stats
    pub trades_count: u32,
    pub total_profit_pct: f64,
    pub last_latency_ms: Option<u64>,
    pub uptime_start: Instant,
    
    // Logs (ring buffer)
    pub recent_logs: VecDeque<LogEntry>,
    
    // Control
    pub should_quit: bool,
    pub log_scroll_offset: usize,
    pub show_debug_logs: bool,
}

impl AppState {
    /// Create new AppState with config values
    pub fn new(
        pair: String,
        spread_entry: f64,
        spread_exit: f64,
        position_size: f64,
        leverage: u32,
    ) -> Self {
        Self {
            vest_best_bid: 0.0,
            vest_best_ask: 0.0,
            paradex_best_bid: 0.0,
            paradex_best_ask: 0.0,
            current_spread_pct: 0.0,
            spread_direction: None,
            position_open: false,
            entry_spread: None,
            entry_direction: None,
            position_polls: 0,
            pair,
            spread_entry_threshold: spread_entry,
            spread_exit_threshold: spread_exit,
            position_size,
            leverage,
            trades_count: 0,
            total_profit_pct: 0.0,
            last_latency_ms: None,
            uptime_start: Instant::now(),
            recent_logs: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            should_quit: false,
            log_scroll_offset: 0,
            show_debug_logs: false,
        }
    }
    
    /// Add a log entry with automatic rotation
    pub fn push_log(&mut self, entry: LogEntry) {
        if self.recent_logs.len() >= MAX_LOG_ENTRIES {
            self.recent_logs.pop_front();
        }
        self.recent_logs.push_back(entry);
    }
    
    /// Get formatted uptime string
    pub fn uptime_str(&self) -> String {
        let elapsed = self.uptime_start.elapsed();
        let hours = elapsed.as_secs() / 3600;
        let minutes = (elapsed.as_secs() % 3600) / 60;
        format!("{}h{:02}m", hours, minutes)
    }
    
    /// Update orderbook prices
    pub fn update_prices(
        &mut self,
        vest_bid: f64,
        vest_ask: f64,
        paradex_bid: f64,
        paradex_ask: f64,
    ) {
        self.vest_best_bid = vest_bid;
        self.vest_best_ask = vest_ask;
        self.paradex_best_bid = paradex_bid;
        self.paradex_best_ask = paradex_ask;
    }
    
    /// Update spread info
    pub fn update_spread(&mut self, spread_pct: f64, direction: Option<SpreadDirection>) {
        self.current_spread_pct = spread_pct;
        self.spread_direction = direction;
    }
    
    /// Record trade entry
    pub fn record_entry(&mut self, spread: f64, direction: SpreadDirection) {
        self.position_open = true;
        self.entry_spread = Some(spread);
        self.entry_direction = Some(direction);
        self.position_polls = 0;
    }
    
    /// Record trade exit
    pub fn record_exit(&mut self, profit_pct: f64, latency_ms: u64) {
        self.position_open = false;
        self.entry_spread = None;
        self.entry_direction = None;
        self.trades_count += 1;
        self.total_profit_pct += profit_pct;
        self.last_latency_ms = Some(latency_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_state_creation() {
        let state = AppState::new(
            "BTC-PERP".to_string(),
            0.15,
            0.05,
            0.001,
            10,
        );
        assert_eq!(state.pair, "BTC-PERP");
        assert_eq!(state.spread_entry_threshold, 0.15);
        assert!(!state.position_open);
        assert!(state.recent_logs.is_empty());
    }
    
    #[test]
    fn test_log_rotation() {
        let mut state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        
        // Fill beyond capacity
        for i in 0..150 {
            state.push_log(LogEntry {
                timestamp: format!("12:00:{:02}", i % 60),
                level: "INFO".to_string(),
                message: format!("Log {}", i),
            });
        }
        
        // Should be capped at MAX_LOG_ENTRIES
        assert_eq!(state.recent_logs.len(), MAX_LOG_ENTRIES);
        
        // First entry should be from i=50 (after rotation)
        assert!(state.recent_logs.front().unwrap().message.contains("50"));
    }
    
    #[test]
    fn test_trade_recording() {
        let mut state = AppState::new("BTC".into(), 0.1, 0.05, 0.001, 10);
        
        state.record_entry(0.12, SpreadDirection::AOverB);
        assert!(state.position_open);
        assert_eq!(state.entry_spread, Some(0.12));
        
        state.record_exit(0.08, 45);
        assert!(!state.position_open);
        assert_eq!(state.trades_count, 1);
        assert_eq!(state.total_profit_pct, 0.08);
        assert_eq!(state.last_latency_ms, Some(45));
    }
}
```

---

### Task 3: Créer TuiLayer (bot/src/tui/logging.rs)

**File:** `bot/src/tui/logging.rs` [NEW]

**Content:**
```rust
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
            level,
            message,
        };
        
        // Use try_lock to avoid blocking hot path
        if let Ok(mut state) = self.app_state.try_lock() {
            // Filter DEBUG if not enabled
            if entry.level == "DEBUG" && !state.show_debug_logs {
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
        // Layer created successfully
        assert!(Arc::strong_count(&layer.app_state) == 2);
    }
}
```

---

### Task 4: Créer Event Handler (bot/src/tui/event.rs)

**File:** `bot/src/tui/event.rs` [NEW]

**Content:**
```rust
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
pub async fn handle_events(
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
```

---

### Task 5: Créer UI Renderer (bot/src/tui/ui.rs)

**File:** `bot/src/tui/ui.rs` [NEW]

**Content:** (fichier complet avec layout 4 zones, couleurs, widgets ratatui)

> Ce fichier fait ~200 lignes avec les imports ratatui, le layout en `Layout::vertical`, et les blocs colorés. Je fournirai le code complet lors de l'implémentation.

**Key elements:**
- `draw(frame: &mut Frame, state: &AppState)` fonction principale
- Header: pair, spread, position status
- Orderbooks: 2 colonnes Vest/Paradex
- Stats: entry, PnL, latency, trades, uptime
- Logs: scrollable avec couleurs par level

---

### Task 6: Créer Module Root (bot/src/tui/mod.rs)

**File:** `bot/src/tui/mod.rs` [NEW]

```rust
//! TUI Module for HFT Bot
//!
//! Optional terminal user interface activated via LOG_FORMAT=tui

pub mod app;
pub mod event;
pub mod logging;
pub mod ui;

pub use app::{AppState, LogEntry};
pub use logging::TuiLayer;
```

---

### Task 7: Modifier lib.rs

**File:** `bot/src/lib.rs`

**Action:** Ajouter `pub mod tui;` avec feature gate conditionnel:

```rust
pub mod adapters;
pub mod config;
pub mod core;
pub mod error;
pub mod tui;  // TUI module (opt-in via LOG_FORMAT=tui)
```

---

### Task 8: Modifier config/logging.rs

**File:** `bot/src/config/logging.rs`

**Action:** Ajouter mode TUI qui retourne le TuiLayer pour composition:

```rust
use std::sync::{Arc, Mutex};
use crate::tui::{AppState, TuiLayer};

/// Logging mode for init
pub enum LoggingMode {
    Json,
    Pretty,
    Tui(Arc<Mutex<AppState>>),
}

/// Initialize logging - returns TuiLayer if TUI mode
pub fn init_logging_with_mode(mode: LoggingMode) -> Option<TuiLayer> {
    match mode {
        LoggingMode::Json => {
            // existing JSON init
            None
        }
        LoggingMode::Pretty => {
            // existing Pretty init
            None
        }
        LoggingMode::Tui(app_state) => {
            // Return TuiLayer for external composition
            Some(TuiLayer::new(app_state))
        }
    }
}
```

---

### Task 9: Modifier main.rs pour TUI

**File:** `bot/src/main.rs`

**Actions:**
1. Détecter `LOG_FORMAT=tui`
2. Si TUI: créer AppState, init terminal, spawn render task
3. Panic hook pour restore terminal
4. Shutdown: restore terminal

**Insertion points:**
- Après L35 (dotenvy): detect TUI mode
- Après L38 (init_logging): conditional TUI setup
- Avant L254 (shutdown): terminal restore

---

### Task 10: Ajouter AppState updates dans runtime.rs

**File:** `bot/src/core/runtime.rs`

**Action:** Passer `Option<Arc<Mutex<AppState>>>` à `execution_task`, mettre à jour sur position open/close.

---

### Task 11: Tests et validation

**Commands:**
```bash
cd bot
cargo build          # Compile sans erreur
cargo test           # 202+ tests passent
cargo clippy         # Pas de warnings
LOG_FORMAT=tui cargo run  # Test manuel TUI
```

---

## Acceptance Criteria

### AC1: Mode headless préservé
- **Given** LOG_FORMAT=json ou non défini
- **When** le bot démarre
- **Then** logs JSON sur stdout comme avant, pas de changement de comportement

### AC2: Mode TUI activé
- **Given** LOG_FORMAT=tui
- **When** le bot démarre
- **Then** terminal en raw mode, TUI affichée avec 4 zones

### AC3: Données temps réel
- **Given** TUI active
- **When** orderbooks mis à jour (25ms)
- **Then** prix bid/ask et spread rafraîchis à 100ms dans l'UI

### AC4: Logs capturés
- **Given** TUI active
- **When** un événement tracing est émis
- **Then** log visible dans la zone logs (max 100, rotation auto)

### AC5: Keyboard handling
- **Given** TUI active
- **When** appui sur 'q' ou Ctrl+C
- **Then** shutdown graceful, terminal restauré

### AC6: Terminal restore on panic
- **Given** TUI active
- **When** panic dans le code
- **Then** terminal restauré (raw mode désactivé, alternate screen quittée)

### AC7: Tests passent
- **Given** code implémenté
- **When** `cargo test`
- **Then** tous les tests existants (202+) + nouveaux tests TUI passent

### AC8: No HFT latency impact
- **Given** TUI active
- **When** monitoring_task poll (25ms)
- **Then** pas de lock blocking (try_lock), latence inchangée

---

## Additional Context

### Dependencies

```toml
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
```

### Testing Strategy

1. **Unit Tests (automatisés):**
   - `AppState::new()`, `push_log()`, `uptime_str()`
   - `TuiLayer` capture verification
   - Log rotation at MAX_LOG_ENTRIES

2. **Manual Tests:**
   - `LOG_FORMAT=tui cargo run --release`
   - Vérifier affichage 4 zones
   - Tester q, Ctrl+C, j/k scroll, l toggle
   - Simuler panic (unwrap sur None) → terminal doit être restauré

### Notes

- **crossterm event-stream** permet async polling sans bloquer tokio
- **ratatui 0.29** est la version stable actuelle
- Le TuiLayer utilise `try_lock()` pour éviter deadlock avec monitoring_task
