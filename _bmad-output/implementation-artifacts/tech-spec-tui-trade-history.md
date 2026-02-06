---
title: 'TUI Trade History Panel'
slug: 'tui-trade-history'
created: '2026-02-06'
status: 'implementation-complete'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, ratatui]
files_to_modify: [src/tui/app.rs, src/tui/ui.rs]
code_patterns: [VecDeque ring buffer, ratatui widgets, Constraint-based layout]
test_patterns: [Unit tests in mod tests blocks]
---

# Tech-Spec: TUI Trade History Panel

**Created:** 2026-02-06

## Overview

### Problem Statement

Les données de trades (spreads) sont perdues à la fermeture de position. L'utilisateur ne peut pas voir l'historique pour analyser les performances.

### Solution

Ajouter une section "Trade History" dans le TUI avec les 10 derniers trades en format compact : `#1 BOverA │ E:+0.34% X:-0.12% │ +0.22%`

### Scope

**In Scope:**
- `TradeRecord` struct + ring buffer 10 trades
- Nouvelle section TUI entre orderbooks et stats
- Format compact avec spreads et PnL

**Out of Scope:**
- Persistance fichier, historique pré-démarrage

## Context for Development

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `src/tui/app.rs` | État, structs, record_entry/exit |
| `src/tui/ui.rs` | Layout + draw functions |

## Implementation Plan

### Tasks

- [x] **Task 1**: Ajouter `TradeRecord` struct dans `app.rs`
  - File: `src/tui/app.rs` (après ligne 19)
  - Action: Créer struct avec `direction`, `entry_spread`, `exit_spread`, `pnl_pct`, `timestamp`

- [x] **Task 2**: Ajouter champs historique à `AppState`
  - File: `src/tui/app.rs` (ligne ~58)
  - Action: Ajouter `trade_history: VecDeque<TradeRecord>` et `MAX_TRADE_HISTORY: usize = 10`

- [x] **Task 3**: Modifier `AppState::new()` pour initialiser `trade_history`
  - File: `src/tui/app.rs` (ligne ~99)
  - Action: Ajouter `trade_history: VecDeque::with_capacity(10)`

- [x] **Task 4**: Modifier `record_exit()` pour sauvegarder l'historique
  - File: `src/tui/app.rs` (ligne 204-214)
  - Action: AVANT reset des champs, créer `TradeRecord` et push au ring buffer

- [x] **Task 5**: Modifier layout dans `draw()`
  - File: `src/tui/ui.rs` (ligne 22-35)
  - Action: Ajouter `Constraint::Length(5)` pour Trade History (entre chunks[1] et Stats)

- [x] **Task 6**: Créer `draw_trade_history()`
  - File: `src/tui/ui.rs` (nouvelle fonction après `draw_orderbooks`)
  - Action: Utiliser `List` widget avec format `#N DIR │ E:+X% X:-Y% │ +Z%`

- [x] **Task 7**: Ajouter unit test
  - File: `src/tui/app.rs` (dans mod tests)
  - Action: Test `test_trade_history_ring_buffer` vérifiant ring buffer

### Acceptance Criteria

- [x] **AC1**: Given bot ferme une position, when `record_exit()` appelé, then `TradeRecord` ajouté à `trade_history`
- [x] **AC2**: Given 11 trades complétés, when `trade_history.len()`, then = 10 (ring buffer)
- [x] **AC3**: Given TUI actif, when écran affiché, then section "Trade History" visible entre orderbooks et stats
- [x] **AC4**: Given trade entry +0.34% exit -0.12%, when affiché, then PnL = +0.46%

## Additional Context

### Dependencies

Aucune nouvelle dépendance.

### Testing Strategy

```powershell
cargo test --release -- tui
$env:LOG_FORMAT="tui"; cargo run --release --bin hft_bot
```

### Notes

- PnL calculé : `entry_spread - exit_spread`
- Timestamp : `chrono::Local::now().format("%H:%M:%S")`
