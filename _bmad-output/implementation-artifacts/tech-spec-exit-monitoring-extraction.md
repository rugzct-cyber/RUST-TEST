---
title: 'Exit Monitoring Extraction'
slug: 'exit-monitoring-extraction'
created: '2026-02-05T00:38:40+01:00'
status: 'implementation-complete'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, tokio, async, tracing]
files_to_modify: [src/core/runtime.rs]
code_patterns: [async-function-extraction, broadcast-receiver-pattern, shared-orderbooks-read]
test_patterns: [mock-adapter, shared-orderbooks, tokio-test]
---

# Tech-Spec: Exit Monitoring Extraction

**Created:** 2026-02-05T00:38:40+01:00

## Overview

### Problem Statement

La boucle `exit_monitoring` (75 lignes, L140-254 de `runtime.rs`) est embedded dans `execution_task`, créant:
- Couplage tight entre exécution et monitoring
- Loop nested difficile à tester isolément
- Complexité excessive de `execution_task` (~200 lignes)

### Solution

Extraire la logique de monitoring de sortie en une fonction standalone `exit_monitoring_loop` qui:
- Prend les mêmes paramètres qu'actuellement
- Préserve le comportement exact (log errors, don't propagate)
- Retourne quand exit condition atteinte ou shutdown reçu
- Réduit `execution_task` de ~200 à ~100 lignes

### Scope

**In Scope:**
- Extraction de L140-254 en fonction standalone
- Signature de fonction avec génériques `<V, P>`
- Préservation du comportement actuel (pas de changement fonctionnel)
- Tests unitaires pour la fonction extraite
- Mise à jour des tests existants si nécessaire

**Out of Scope:**
- Mode "warm start" / restore positions
- Mode "exit-only" standalone
- Changement de comportement (propagation d'erreurs)
- Refactorisation de `execution.rs`

## Context for Development

### Codebase Patterns

- **Async function extraction**: Suivre le pattern de `drain_channel()` (L43-51) déjà extrait dans runtime.rs
- **Broadcast receiver**: Le `shutdown_rx` doit être passé par `&mut` car `recv()` consomme
- **SharedOrderbooks**: Utiliser `Arc<RwLock<HashMap>>` pattern existant avec `.read().await.get().cloned()`
- **Generic bounds**: `V: ExchangeAdapter + Send + Sync, P: ExchangeAdapter + Send + Sync`
- **TradingEvent factory**: Utiliser les méthodes factory existantes (`position_monitoring`, `trade_exit`, `position_closed`)

### Dependencies (Step 2 Investigation)

**Imports requis pour la nouvelle fonction:**
```rust
// Already imported in runtime.rs - no changes needed
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};
use crate::core::events::{TradingEvent, log_event, format_pct};
use crate::core::execution::DeltaNeutralExecutor;
use crate::core::spread::{SpreadCalculator, SpreadDirection};
```

**Functions/Methods called by exit loop:**
| Call | Source | Line |
|------|--------|------|
| `SharedOrderbooks.read().await` | tokio RwLock | L169-170 |
| `Orderbook.best_bid()`, `best_ask()` | crate::adapters | L174-177 |
| `SpreadCalculator::calculate_exit_spread()` | crate::core::spread | L184, L189 |
| `TradingEvent::position_monitoring()` | crate::core::events | L195-202 |
| `TradingEvent::trade_exit()` | crate::core::events | L210-218 |
| `TradingEvent::position_closed()` | crate::core::events | L224-230 |
| `executor.close_position()` | crate::core::execution | L221 |
| `log_event()` | crate::core::events | L202, L218, L230 |

### Files to Reference

| File | Purpose |
| ---- | ------- |
| [runtime.rs](file:///c:/Users/jules/Documents/bot4/src/core/runtime.rs) | Source du code à extraire (L140-254) |
| [execution.rs](file:///c:/Users/jules/Documents/bot4/src/core/execution.rs) | `DeltaNeutralExecutor`, `close_position()` |
| [spread.rs](file:///c:/Users/jules/Documents/bot4/src/core/spread.rs) | `SpreadCalculator::calculate_exit_spread()` |

### Technical Decisions

1. **Placement**: Rester dans `runtime.rs` (pas de changement d'architecture)
2. **Retour**: `u64` (poll_count) pour le log final - pas de propagation d'erreurs
3. **Scope**: Extraction pure, pas de hooks pour warm start

### Red Team Analysis (V1-V5)

| ID | Sévérité | Faille dans proposition originale | Fix appliqué |
|----|----------|-----------------------------------|--------------|
| V1 | 🔴 CRITICAL | `entry_direction` manquant | Passer `direction: SpreadDirection` explicitement |
| V2 | 🟡 MEDIUM | `pair: String` cloné inutilement | `pair: &str` |
| V3 | 🟡 MEDIUM | Symbols clonés | `vest_symbol: &str, paradex_symbol: &str` |
| V4 | 🟢 LOW | Return type `Result<(), ExchangeError>` menteur | Retourner `u64` (poll_count) |
| V5 | 🟡 MEDIUM | `poll_count` perdu après loop | Inclus dans V4 |

### Consolidated Function Signature

```rust
/// Exit monitoring loop - polls orderbooks until exit condition or shutdown
/// 
/// Returns: poll_count for final logging
async fn exit_monitoring_loop<V, P>(
    executor: &DeltaNeutralExecutor<V, P>,
    vest_orderbooks: SharedOrderbooks,
    paradex_orderbooks: SharedOrderbooks,
    vest_symbol: &str,
    paradex_symbol: &str,
    pair: &str,
    entry_spread: f64,
    exit_spread_target: f64,
    direction: SpreadDirection,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> u64
where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
```

## Implementation Plan

### Tasks

- [x] **Task 1: Create exit_monitoring_loop function signature**
  - File: `src/core/runtime.rs`
  - Action: Add new async function after `drain_channel()` helper (after L51)
  - Code: Implement the consolidated signature from Red Team analysis
  - Notes: Use `#[allow(clippy::too_many_arguments)]` like `execution_task`

- [x] **Task 2: Extract exit loop body into new function**
  - File: `src/core/runtime.rs`
  - Action: Move L153-252 (interval setup + 'exit_loop block) into `exit_monitoring_loop`
  - Notes: 
    - Replace `spread_pct` with `entry_spread` parameter
    - Replace direct `direction` binding with parameter
    - Return `poll_count` at end

- [x] **Task 3: Update execution_task to call extracted function**
  - File: `src/core/runtime.rs`
  - Action: Replace L143-254 with call to `exit_monitoring_loop`
  - Code:
    ```rust
    if let Some(direction) = executor.get_entry_direction() {
        debug!(
            event_type = "POSITION_OPENED",
            direction = ?direction,
            exit_target = %format_pct(exit_spread_target),
            "Starting exit monitoring"
        );
        
        let poll_count = exit_monitoring_loop(
            &executor,
            vest_orderbooks.clone(),
            paradex_orderbooks.clone(),
            &vest_symbol,
            &paradex_symbol,
            &pair,
            spread_pct,
            exit_spread_target,
            direction,
            &mut shutdown_rx,
        ).await;
        
        info!(event_type = "POSITION_MONITORING", total_polls = poll_count, "Exit monitoring stopped");
    } else {
        error!(event_type = "ORDER_FAILED", "No entry direction found after successful trade");
    }
    ```

- [x] **Task 4: Add unit test for exit_monitoring_loop directly**
  - File: `src/core/runtime.rs`
  - Action: Add new test `test_exit_monitoring_loop_exits_on_spread_condition`
  - Notes: Test the extracted function in isolation with mock orderbooks that trigger exit immediately

- [x] **Task 5: Verify existing tests still pass**
  - Action: Run `cargo test --lib`
  - Notes: `test_execution_task_processes_opportunity` and `test_execution_task_shutdown` must still pass

### Acceptance Criteria

- [x] **AC1**: Given `exit_monitoring_loop` is called, when `exit_spread >= exit_spread_target`, then the function closes the position and returns the poll_count
- [x] **AC2**: Given `exit_monitoring_loop` is running, when `shutdown_rx` receives signal, then the function breaks immediately and returns current poll_count
- [x] **AC3**: Given the extraction is complete, when `cargo build` runs, then compilation succeeds with no new warnings
- [x] **AC4**: Given the extraction is complete, when `cargo test --lib` runs, then all existing tests (165 now) pass
- [x] **AC5**: Given the extraction is complete, when reviewing `execution_task`, then function is reduced by ~60 lines (from ~200 to ~140)

## Additional Context

### Dependencies

- **Internal**: No new dependencies - all imports already present
- **External**: None
- **Blocking**: None - pure refactoring

### Testing Strategy

**Automated Tests:**
```bash
# Run all library tests
cargo test --lib

# Run only runtime tests
cargo test runtime::tests
```

**Verification Commands:**
1. `cargo build` - Verify compilation
2. `cargo clippy` - No new warnings
3. `cargo test --lib` - All 163+ tests pass

**Manual Verification:**
- None required - automated tests provide full coverage

### Notes

- The existing test `test_execution_task_processes_opportunity` already tests the exit flow via integration
- New unit test for `exit_monitoring_loop` improves isolation testing
- Red Team V1-V5 fixes are critical - DO NOT skip the direction parameter

