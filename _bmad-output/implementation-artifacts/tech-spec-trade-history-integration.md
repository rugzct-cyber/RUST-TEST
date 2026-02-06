---
title: 'Intégration Trade History dans le flow d''exécution'
slug: 'trade-history-integration'
created: '2026-02-06'
status: 'implementation-complete'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, ratatui, tokio, chrono]
files_to_modify: [src/core/runtime.rs, src/tui/app.rs, src/tui/ui.rs]
code_patterns: [Arc<Mutex<AppState>> partagé, exit_monitoring_loop return, DeltaNeutralExecutor getter]
test_patterns: [Unit tests in mod tests blocks, cargo test --release -- tui, cargo test --release -- runtime]
---

# Tech-Spec: Intégration Trade History dans le flow d'exécution

**Created:** 2026-02-06

## Overview

### Problem Statement

L'infrastructure Trade History (`TradeRecord`, ring buffer, panneau TUI) est implémentée mais `record_exit()` n'est jamais appelé depuis `runtime.rs`. `record_entry()` est déjà connecté (runtime.rs:292). Le PnL est en % alors que l'utilisateur attend un PnL en dollars.

### Solution

Connecter `record_exit()` au flow d'exécution en modifiant `exit_monitoring_loop` pour retourner l'`exit_spread`, puis appeler `record_exit()` dans `execution_task` après la boucle. Convertir le PnL en $.

### Scope

**In Scope:**
- Modifier `exit_monitoring_loop` return type pour inclure `exit_spread`
- Appeler `record_exit()` dans `execution_task`
- Convertir `pnl_pct` → `pnl_usd` dans `TradeRecord` et `AppState`
- Adapter l'affichage TUI
- Mettre à jour les tests existants

**Out of Scope:**
- Récupération de positions au démarrage
- Persistance de l'historique
- `record_entry()` (déjà connecté)

## Context for Development

### Codebase Patterns

- `tui_state: Option<Arc<StdMutex<TuiState>>>` partagé via Arc+Mutex
- `exit_monitoring_loop` retourne actuellement `u64` (poll_count)
- Pattern TUI update : `if let Some(ref tui) = tui_state { tui.try_lock() }`
- `executor.get_default_quantity()` retourne `position_size` (tokens)
- `entry_vest_price` / `entry_paradex_price` stockés dans `AppState`

### Files to Reference

| File | Purpose | Lignes clés |
| ---- | ------- | ----------- |
| `src/core/runtime.rs` | `execution_task()` L209-358, `exit_monitoring_loop()` L57-187 | L292 record_entry, L312-323 exit loop |
| `src/tui/app.rs` | `TradeRecord` L24-32, `record_exit()` L222-248 | L30 pnl_pct, L246 total_profit_pct |
| `src/tui/ui.rs` | `draw_trade_history()` | Format PnL |
| `src/core/execution.rs` | `get_default_quantity()` L325-327 | position_size getter |

### Technical Decisions

1. **record_exit dans execution_task** : Après `exit_monitoring_loop`, pas à l'intérieur (accès garanti à `tui_state`)
2. **PnL en $** : `(entry_spread_pct + exit_spread_pct) / 100.0 × avg_entry_price × position_size`
3. **Return type étendu** : `(u64, Option<f64>)` - `None` pour shutdown, `Some(exit_spread)` pour exit normal

## Implementation Plan

### Tasks

- [x] Task 1: Modifier le return type de `exit_monitoring_loop`
  - File: `src/core/runtime.rs`
  - Action: Changer la signature de `-> u64` à `-> (u64, Option<f64>)`
  - Détails:
    - Ligne 72 : changer `) -> u64` en `) -> (u64, Option<f64>)`
    - Ligne 84 (shutdown break) : changer `break;` en `break;` et à la fin retourner `(poll_count, None)` au lieu de `poll_count`
    - Ligne 137-173 (exit condition) : stocker `exit_spread` avant le break, et retourner `(poll_count, Some(exit_spread))`
    - Ligne 186 : changer `poll_count` en `(poll_count, None)` pour le cas shutdown par défaut
  - Notes: Le `exit_spread` est déjà calculé ligne 101-112, il suffit de le capturer

- [x] Task 2: Modifier `TradeRecord` et `record_exit()` pour PnL en $
  - File: `src/tui/app.rs`
  - Action: Renommer `pnl_pct` → `pnl_usd` et `total_profit_pct` → `total_profit_usd`
  - Détails:
    - `TradeRecord.pnl_pct` (L30) → `pnl_usd: f64`
    - `AppState.total_profit_pct` → `total_profit_usd: f64`
    - `record_exit()` (L222) : renommer paramètre `profit_pct` → `pnl_usd`
    - L229 : `pnl_pct: profit_pct` → `pnl_usd: pnl_usd`
    - L246 : `self.total_profit_pct += profit_pct` → `self.total_profit_usd += pnl_usd`

- [x] Task 3: Appeler `record_exit()` dans `execution_task`
  - File: `src/core/runtime.rs`
  - Action: Après `exit_monitoring_loop`, appeler `record_exit` via `tui_state`
  - Détails:
    - L312 : changer `let _poll_count = exit_monitoring_loop(...)` en `let (poll_count, maybe_exit_spread) = exit_monitoring_loop(...)`
    - Après L323 (après `log_system_event`), ajouter :
      ```rust
      // Update TUI trade history
      if let (Some(exit_spread), Some(ref tui)) = (maybe_exit_spread, &tui_state) {
          if let Ok(mut state) = tui.try_lock() {
              // PnL = spread convergence × avg_price × position_size
              let avg_price = match (state.entry_vest_price, state.entry_paradex_price) {
                  (Some(v), Some(p)) if v > 0.0 && p > 0.0 => (v + p) / 2.0,
                  (Some(v), _) if v > 0.0 => v,
                  (_, Some(p)) if p > 0.0 => p,
                  _ => 0.0,
              };
              let position_size = executor.get_default_quantity();
              let pnl_usd = (spread_pct + exit_spread) / 100.0 * avg_price * position_size;
              let latency_ms = poll_count * 25; // EXIT_POLL_INTERVAL_MS
              state.record_exit(exit_spread, pnl_usd, latency_ms);
          }
      }
      ```
  - Notes: `spread_pct` est le entry_spread déjà disponible dans le scope local

- [x] Task 4: Adapter l'affichage TUI
  - File: `src/tui/ui.rs`
  - Action: Modifier `draw_trade_history()` pour afficher PnL en $
  - Détails:
    - Changer le format de chaque trade : `pnl_pct` → `pnl_usd`
    - Format : `${pnl_usd:+.2}` au lieu de `{pnl_pct:+.2}%`
    - Stats panel : si `total_profit_pct` est affiché, le changer en `total_profit_usd` avec format `$`

- [x] Task 5: Mettre à jour les tests
  - File: `src/tui/app.rs`
  - Action: Adapter les tests existants pour les renommages
  - Détails:
    - `test_trade_recording` : vérifier `total_profit_usd` au lieu de `total_profit_pct`
    - `test_trade_history_recording` : vérifier `pnl_usd` au lieu de `pnl_pct`
  - File: `src/core/runtime.rs`
  - Action: Adapter les tests pour le nouveau return type de `exit_monitoring_loop`
  - Détails:
    - `test_exit_monitoring_loop_exits_on_spread_condition` : déstructurer `(poll_count, exit_spread)` au lieu de `poll_count`
    - Vérifier que `exit_spread.is_some()`
    - `test_exit_monitoring_loop_responds_to_shutdown` : vérifier exit_spread est `None`
    - `test_exit_monitoring_loop_b_over_a_direction` : même pattern

### Acceptance Criteria

- [x] AC1: Given une position ouverte via `execution_task`, when `exit_monitoring_loop` détecte la condition de sortie et `close_position` réussit, then `record_exit()` est appelé et un `TradeRecord` avec `pnl_usd` apparaît dans `trade_history`

- [x] AC2: Given un `TradeRecord` dans `trade_history`, when le TUI render le panneau Trade History, then le PnL est affiché au format `$X.XX` (vert si positif, rouge si négatif)

- [x] AC3: Given `exit_monitoring_loop` interrompu par shutdown (pas par exit condition), when la boucle se termine, then `record_exit()` n'est PAS appelé (pas de faux trade)

- [x] AC4: Given le code modifié, when `cargo test --release -- tui`, then tous les tests passent

- [x] AC5: Given le code modifié, when `cargo test --release -- runtime`, then tous les tests passent (y compris le nouveau return type)

## Additional Context

### Dependencies

- Infrastructure Trade History déjà implémentée (`TradeRecord`, ring buffer, `draw_trade_history`)
- `chrono` crate (déjà utilisé)
- `executor.get_default_quantity()` déjà public

### Testing Strategy

- **Tests automatisés :**
  - `cargo test --release -- tui` : valide `record_exit`, `TradeRecord`, ring buffer
  - `cargo test --release -- runtime` : valide `exit_monitoring_loop` return type, shutdown vs exit
- **Vérification manuelle :**
  - Lancer le bot en mode TUI (`LOG_FORMAT=tui cargo run --release`)
  - Observer qu'après une entrée/sortie de position, une ligne apparaît dans le panneau Trade History avec le PnL en $

### Notes

- `record_entry()` est déjà intégré — pas besoin de le toucher
- L'ordre des tâches est critique : Task 1 d'abord (runtime return type), puis Task 2 (app.rs renaming), puis Task 3 (branchement dans runtime), Task 4 (UI), Task 5 (tests)
- Le PnL est une **approximation** basée sur les spreads, pas un calcul exact depuis les prix de fill (acceptable pour V1)
