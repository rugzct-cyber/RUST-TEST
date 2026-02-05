---
title: 'Log Centralization & Coherence'
slug: 'log-centralization'
created: '2026-02-05'
status: 'completed'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, tracing, tracing-subscriber]
files_to_modify: [src/core/events.rs, src/core/runtime.rs, src/core/execution.rs, src/core/monitoring.rs]
code_patterns: [debug!, info!, log_system_event, SystemEvent]
test_patterns: [cargo_test, manual_terminal, grep_verification]
---

# Tech-Spec: Log Centralization & Coherence

**Created:** 2026-02-05

## Overview

### Problem Statement

1. **Logs dispersés** : Les logs system sont éparpillés dans 3 fichiers, difficiles à maintenir
2. **Cohérence des niveaux** : En mode `RUST_LOG=info`, trop de bruit technique noie les infos trading

### Solution

Centraliser TOUS les logs system dans `events.rs` via un nouveau type `SystemEvent`, avec les bons niveaux (DEBUG/INFO) dès la création.

> [!IMPORTANT]
> **Fusion des phases** : Pas de Phase 1 séparée. On migre directement vers `log_system_event()` avec les niveaux corrects.

### Scope

**In Scope:**
- Création de `SystemEvent` enum et factories dans events.rs
- Migration des logs dispersés vers `log_system_event()`
- Niveaux corrects dès la migration (DEBUG pour bruit, INFO pour lifecycle)

**Out of Scope:**
- Logs dans les adapters (`paradex/*.rs`, `vest/*.rs`)
- Logs dans `pyth.rs`
- Error logs (`error!()`) - restent inline

## Context for Development

### Architecture Cible

```
events.rs
├── TradingEvent (existant)
│   ├── SpreadDetected, TradeEntry, TradeHold, TradeExit, Slippage
│   └── log_event() → format compact [TAG]
│
└── SystemEvent (nouveau)
    ├── TaskStarted, TaskStopped, TaskShutdown
    ├── AdapterReconnect, PositionVerified, PositionDetail, TradeStarted
    └── log_system_event() → format structuré verbose
```

### Log Format Specification

**SystemEvent format (verbose, structured):**
```
# DEBUG level example
2026-02-05T19:30:00 DEBUG event_type="TASK_STARTED" task="execution" "Execution task started"

# INFO level example  
2026-02-05T19:35:00 INFO event_type="TASK_SHUTDOWN" task="monitoring" reason="user_signal" "Monitoring task shutting down"
```

### Technical Decisions

1. **Niveaux par SystemEventType:**
   - `TaskStarted` → DEBUG
   - `TaskStopped` → INFO
   - `TaskShutdown` → INFO
   - `AdapterReconnect` → DEBUG
   - `PositionVerified` → DEBUG
   - `PositionDetail` → DEBUG
   - `TradeStarted` → DEBUG

2. **Error logs restent inline** : `error!()` calls ne sont pas migrés (ils sont rares et contextuels)

## Implementation Plan

### T1: Créer `SystemEvent` dans `events.rs`

**File**: `src/core/events.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SystemEventType {
    TaskStarted,
    TaskStopped,
    TaskShutdown,
    AdapterReconnect,
    PositionVerified,
    PositionDetail,
    TradeStarted,
}

pub struct SystemEvent {
    pub event_type: SystemEventType,
    pub task_name: Option<String>,
    pub exchange: Option<String>,
    pub message: String,
    pub details: Option<String>,
}
```

### T2: Créer les factories pour `SystemEvent`

```rust
impl SystemEvent {
    pub fn task_started(task_name: &str) -> Self {
        Self {
            event_type: SystemEventType::TaskStarted,
            task_name: Some(task_name.to_string()),
            exchange: None,
            message: format!("{} task started", task_name),
            details: None,
        }
    }

    pub fn task_stopped(task_name: &str) -> Self {
        Self {
            event_type: SystemEventType::TaskStopped,
            task_name: Some(task_name.to_string()),
            exchange: None,
            message: format!("{} task stopped", task_name),
            details: None,
        }
    }

    pub fn task_shutdown(task_name: &str, reason: &str) -> Self {
        Self {
            event_type: SystemEventType::TaskShutdown,
            task_name: Some(task_name.to_string()),
            exchange: None,
            message: format!("{} shutting down", task_name),
            details: Some(reason.to_string()),
        }
    }

    pub fn adapter_reconnect(exchange: &str, status: &str) -> Self {
        Self {
            event_type: SystemEventType::AdapterReconnect,
            task_name: None,
            exchange: Some(exchange.to_string()),
            message: format!("Adapter {}", status),
            details: None,
        }
    }

    pub fn position_verified(
        vest_price: f64,
        paradex_price: f64,
        captured_spread: f64,
    ) -> Self {
        Self {
            event_type: SystemEventType::PositionVerified,
            task_name: None,
            exchange: None,
            message: "Entry positions verified".to_string(),
            details: Some(format!(
                "vest={:.2} paradex={:.2} spread={:.4}%",
                vest_price, paradex_price, captured_spread
            )),
        }
    }

    pub fn position_detail(
        exchange: &str,
        side: &str,
        quantity: f64,
        entry_price: f64,
    ) -> Self {
        Self {
            event_type: SystemEventType::PositionDetail,
            task_name: None,
            exchange: Some(exchange.to_string()),
            message: "Position details".to_string(),
            details: Some(format!(
                "side={} qty={:.4} price={:.2}",
                side, quantity, entry_price
            )),
        }
    }

    pub fn trade_started() -> Self {
        Self {
            event_type: SystemEventType::TradeStarted,
            task_name: None,
            exchange: None,
            message: "Position lock acquired - executing delta-neutral trade".to_string(),
            details: None,
        }
    }
}
```

### T3: Créer `log_system_event()` dans `events.rs`

```rust
pub fn log_system_event(event: &SystemEvent) {
    let event_type_str = format!("{:?}", event.event_type).to_uppercase();
    
    match event.event_type {
        // INFO level events
        SystemEventType::TaskStopped | SystemEventType::TaskShutdown => {
            if let Some(ref details) = event.details {
                info!(
                    event_type = %event_type_str,
                    task = ?event.task_name,
                    details = %details,
                    "{}", event.message
                );
            } else {
                info!(
                    event_type = %event_type_str,
                    task = ?event.task_name,
                    "{}", event.message
                );
            }
        }
        // DEBUG level events
        _ => {
            if let Some(ref exchange) = event.exchange {
                debug!(
                    event_type = %event_type_str,
                    exchange = %exchange,
                    details = ?event.details,
                    "{}", event.message
                );
            } else {
                debug!(
                    event_type = %event_type_str,
                    task = ?event.task_name,
                    details = ?event.details,
                    "{}", event.message
                );
            }
        }
    }
}
```

### T4: Migrer les logs dans `runtime.rs`

**Search for**: `info!` calls avec event_type ou messages de task lifecycle
**Replace with**: `log_system_event(&SystemEvent::xxx())`

| Current Pattern | Replacement |
|-----------------|-------------|
| `info!("Execution task started")` | `log_system_event(&SystemEvent::task_started("execution"))` |
| `info!(..., "Execution task shutting down")` | `log_system_event(&SystemEvent::task_shutdown("execution", "shutdown_signal"))` |
| `info!("Execution task stopped")` | `log_system_event(&SystemEvent::task_stopped("execution"))` |
| `info!(..., "Processing spread opportunity")` | `log_system_event(&SystemEvent::task_started("spread_processing"))` |
| `info!(event_type="POSITION_MONITORING", ...)` | `log_system_event(&SystemEvent::task_stopped("exit_monitoring"))` |

### T5: Migrer les logs dans `execution.rs`

| Current Pattern | Replacement |
|-----------------|-------------|
| `info!(event_type="POSITION_VERIFIED", ...)` | `log_system_event(&SystemEvent::position_verified(vest_price, paradex_price, spread))` |
| `info!(event_type="ADAPTER_RECONNECT", ...)` | `log_system_event(&SystemEvent::adapter_reconnect(exchange, status))` |
| `info!(event_type="TRADE_STARTED", ...)` | `log_system_event(&SystemEvent::trade_started())` |
| `info!(event_type="POSITION_DETAIL", ...)` | `log_system_event(&SystemEvent::position_detail(exchange, side, qty, price))` |

### T6: Migrer les logs dans `monitoring.rs`

| Current Pattern | Replacement |
|-----------------|-------------|
| `info!("Monitoring task started ...")` | `log_system_event(&SystemEvent::task_started("monitoring"))` |
| `info!("Monitoring task shutting down")` | `log_system_event(&SystemEvent::task_shutdown("monitoring", "shutdown_signal"))` |
| `info!("Monitoring task stopped")` | `log_system_event(&SystemEvent::task_stopped("monitoring"))` |

### T7: Vérification

- [ ] `cargo build --release && cargo test`
- [ ] Grep verification: `grep -r "info\!.*event_type" src/core/` should return 0 matches (except events.rs)
- [ ] Manual test INFO mode: only [SCAN], [ENTRY], [HOLD], [EXIT] visible
- [ ] Manual test DEBUG mode: all SystemEvents visible

## Acceptance Criteria

- [ ] **AC1**: `RUST_LOG=info` → NO "task started" messages
- [ ] **AC2**: `RUST_LOG=info` → NO ADAPTER_RECONNECT
- [ ] **AC3**: `RUST_LOG=debug` → ALL SystemEvents visible
- [ ] **AC4**: `cargo test` passes (172+ tests)
- [ ] **AC5**: Mode INFO shows only: `[SCAN]` → `[ENTRY]` → `[HOLD]` → `[EXIT]`
- [ ] **AC6**: `grep -r "info\!.*event_type" src/core/{runtime,execution,monitoring}.rs` returns 0 matches

## Notes

- Error logs (`error!()`) restent inline - pas de migration
- Adapters hors scope (phase future)
