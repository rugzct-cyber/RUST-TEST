---
title: 'Refactoring Format Helpers & Channel Draining'
slug: 'refactor-format-helpers-drain-channel'
created: '2026-02-04T17:05:00+01:00'
status: 'completed'
stepsCompleted: [1, 2, 3, 4]
completionNotes: 'Tasks 1-7 done. Bin/ deferred. Review: 10 findings, 3 fixed (F3,F4,F9), 7 skipped.'
tech_stack: [Rust 1.75+, tokio 1.x, tracing, mpsc channels]
files_to_modify:
  - src/core/mod.rs (add export)
  - src/core/events.rs (add fmt_price)
  - src/core/runtime.rs (add drain_channel, replace patterns)
  - src/core/execution.rs (replace format patterns)
  - src/core/monitoring.rs (replace format patterns)
  - src/main.rs (replace format patterns)
  - src/bin/test_order.rs
  - src/bin/monitor.rs
  - src/bin/delta_neutral_cycle.rs
  - src/bin/test_auto_close.rs
  - src/bin/close_positions.rs
  - src/bin/test_paradex_order.rs
code_patterns:
  - Existing format_pct in events.rs:365 (pub but NOT exported in mod.rs)
  - Tracing structured logging with %format!(...) syntax
  - Explicit re-exports in core/mod.rs
test_patterns:
  - cargo test (unit tests in each module)
  - cargo build --bins (compilation validation for bin/)
---

# Tech-Spec: Refactoring Format Helpers & Channel Draining

**Created:** 2026-02-04T17:05:00+01:00

## Overview

### Problem Statement

1. **Format String Duplication** — 30+ occurrences de `format!("{:.4}%", ...)` et `format!("${:.2}", ...)` dispersées dans le codebase. Cela viole DRY et rend les changements de formatage coûteux.

2. **Channel Draining Pattern** — Le pattern de draining de channel dans `runtime.rs:254-260` et `runtime.rs:267` est dupliqué manuellement au lieu d'être extrait dans une fonction helper.

### Solution

1. **Réutiliser `format_pct`** — La fonction existe déjà dans `events.rs:365-367`. Il suffit de l'exporter publiquement et de remplacer toutes les occurrences.

2. **Ajouter `fmt_price`** — Nouveau helper inline pour le format prix `${:.2}`.

3. **Ajouter `drain_channel<T>`** — Helper générique pour drainer un channel mpsc et logger le nombre de messages drainés.

### Scope

**In Scope:**
- Extraction et réutilisation de `format_pct` dans core/ et bin/
- Nouveau helper `fmt_price` pour les formats prix
- Nouveau helper `drain_channel<T>` dans `runtime.rs`
- Remplacement de toutes les occurrences identifiées

**Out of Scope:**
- Modification des formats dans `vest/adapter.rs` et `vest/signing.rs` (sérialisation API)
- Optimisations de performance ou caching

## Context for Development

### Codebase Patterns

- **Logging structuré via tracing** — Les spreads sont loggués avec `spread = %format_pct(value)`
- **Module events.rs** — Centralise les helpers de formatage et événements de trading
- **Runtime architecture** — `execution_task` gère les opportunities via `mpsc::Receiver`
- **Explicit re-exports** — `core/mod.rs` utilise des exports explicites, pas de glob (`pub use *`)

### Files to Reference

| File | Purpose | Occurrences |
| ---- | ------- | ----------- |
| `src/core/events.rs:365-367` | Existing `format_pct` helper (pub) | Définition |
| `src/core/mod.rs:43` | Re-exports events — **MANQUE format_pct** | À modifier |
| `src/core/runtime.rs:91,134,254-260,267` | format + drain patterns | 4 occurrences |
| `src/core/execution.rs:99,383,437-442,555` | format patterns | 7 occurrences |
| `src/core/monitoring.rs:112-113` | format patterns | 2 occurrences |
| `src/main.rs:74-75` | spread config display `{:.2}%` | 2 occurrences |
| `src/bin/*.rs` | Test scripts avec `${:.2}` | ~20 occurrences |

### Technical Decisions

1. **Placement des helpers prix** — Dans `events.rs` à côté de `format_pct` pour cohérence
2. **Généricité de drain_channel** — Signature `fn drain_channel<T>(rx: &mut mpsc::Receiver<T>, context: &str) -> usize`
3. **Export public** — `format_pct` et `fmt_price` exposés via `pub use` dans `core/mod.rs`
4. **Découverte critique** — `format_pct` est `pub` mais **non exporté** dans `mod.rs:43` → Task 1 doit l'ajouter

## Implementation Plan

### Tasks

- [x] **Task 1**: Exporter `format_pct` dans mod.rs
  - File: `src/core/mod.rs:43`
  - Action: Ajouter `format_pct` et `fmt_price` à la ligne d'export events

- [x] **Task 2**: Ajouter `fmt_price` helper
  - File: `src/core/events.rs`
  - Action: Ajouter après `format_pct` (ligne ~368)
  - Code:
    ```rust
    /// Format price with 2 decimals and $ prefix
    #[inline]
    pub fn fmt_price(value: f64) -> String {
        format!("${:.2}", value)
    }
    ```

- [x] **Task 3**: Créer `drain_channel<T>` helper
  - File: `src/core/runtime.rs`
  - Action: Ajouter après les constantes (ligne ~38)
  - Code:
    ```rust
    /// Drain all pending messages from a channel and log if any were drained
    fn drain_channel<T>(rx: &mut mpsc::Receiver<T>, context: &str) -> usize {
        let mut drained = 0;
        while rx.try_recv().is_ok() {
            drained += 1;
        }
        if drained > 0 {
            debug!("Drained {} stale messages from {}", drained, context);
        }
        drained
    }
    ```

- [x] **Task 4**: Remplacer `format!("{:.4}%", ...)` dans core/
  - Files: `runtime.rs:91,134`, `execution.rs:99,383,440-442,555`, `monitoring.rs:112-113`
  - Action: Remplacer par `format_pct(value)`
  - Note: 13 occurrences totales

- [x] **Task 5**: Remplacer `format!("${:.2}", ...)` dans execution.rs
  - File: `src/core/execution.rs:437-438`
  - Action: Remplacer par `fmt_price(value)`
  - Note: 2 occurrences

- [x] **Task 6**: Remplacer channel draining inline par helper
  - File: `src/core/runtime.rs:254-260, 267`
  - Action: Remplacer par `drain_channel(&mut opportunity_rx, "opportunity queue");`

- [x] **Task 7**: Remplacer dans main.rs
  - File: `src/main.rs:74-75`
  - Action: Importer et remplacer 2 occurrences `{:.2}%`

- [~] **Task 8**: Remplacer dans bin/test_order.rs (deferred - compound strings)
  - File: `src/bin/test_order.rs`
  - Action: Importer `fmt_price` via `use bot4::core::events::fmt_price;`

- [~] **Task 9**: Remplacer dans bin/monitor.rs (deferred - no patterns found)
  - File: `src/bin/monitor.rs`
  - Action: Importer `fmt_price`, remplacer occurrences

- [~] **Task 10**: Remplacer dans bin/delta_neutral_cycle.rs (deferred)
  - File: `src/bin/delta_neutral_cycle.rs`
  - Action: Importer `fmt_price`, remplacer occurrences

- [~] **Task 11**: Remplacer dans bin/test_auto_close.rs (deferred)
  - File: `src/bin/test_auto_close.rs`
  - Action: Importer `fmt_price`, remplacer occurrences

- [~] **Task 12**: Remplacer dans bin/close_positions.rs (deferred)
  - File: `src/bin/close_positions.rs`
  - Action: Importer `fmt_price`, remplacer occurrences

- [~] **Task 13**: Remplacer dans bin/test_paradex_order.rs (deferred)
  - File: `src/bin/test_paradex_order.rs`
  - Action: Importer `fmt_price`, remplacer occurrences

### Acceptance Criteria

```gherkin
Given the codebase uses format_pct and fmt_price helpers
When I search for "format!(\"{:.4}%\"" in src/
Then I find 0 occurrences in core/ files

Given drain_channel helper exists
When I search for "while.*try_recv.*is_ok" in runtime.rs
Then I find only the helper implementation, not inline usage

Given all changes are applied
When I run cargo build
Then the build succeeds with no new warnings

Given all changes are applied
When I run cargo test
Then all tests pass

Given fmt_price helper exists
When I search for "format!(\"${:.2}\"" in src/core/
Then I find 0 occurrences

Given all bin/ files are modified
When I run cargo build --bins
Then all binaries compile successfully
```

## Additional Context

### Dependencies

- Aucune nouvelle dépendance

### Testing Strategy

- **Compilation core**: `cargo build` doit réussir sans warnings
- **Compilation bin/**: `cargo build --bins` valide les binaires (pas de tests unitaires pour bin/)
- **Tests unitaires**: `cargo test` doit passer (régression)
- **Vérification manuelle**: Grep pour confirmer l'élimination des patterns

### Notes

- Le helper `format_pct` existe déjà et est utilisé dans `log_event()` — validation que le format est correct
- Risque global: **1-2/10** — refactoring mécanique sans changement de comportement
