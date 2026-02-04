---
title: 'Code Quality Refactoring - Slippage Constants & Events Factory'
slug: 'code-quality-refactoring-v1'
created: '2026-02-04'
completed: '2026-02-04'
status: 'done'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, tokio, tracing]
files_to_modify: [execution.rs, events.rs]
code_patterns: [module-level-constants, helper-method-factory, tracing-structured-logging]
test_patterns: [cargo-test, inline-mod-tests]
---

# Tech-Spec: Code Quality Refactoring - Slippage Constants & Events Factory

**Created:** 2026-02-04

## Overview

### Problem Statement

Le codebase du bot de trading présente des patterns de duplication qui impactent la maintenabilité:

1. **Constantes de slippage dupliquées** (`execution.rs:548,556`): Deux constantes identiques `VEST_SLIPPAGE_BUFFER` et `PARADEX_SLIPPAGE_BUFFER` définies localement dans `create_orders()`, valeur 0.005 (0.5%).

2. **Factory methods répétitives** (`events.rs:187-457`): 10 méthodes factory qui répètent 18 champs de struct dont 6+ sont systématiquement `None`. ~270 lignes de duplication.

3. **Format strings répétitifs** (`execution.rs`): 14+ occurrences de `format!("{:.4}%", ...)` dispersées.

### Solution

Refactoring en 3 étapes à risque croissant:

1. **Étape 1** (Risque 2/10): Unifier les constantes de slippage au niveau module
2. **Étape 2** (Risque 3/10): Simplifier les factory methods avec un helper `with_pair()`
3. **Étape 3** (Risque 1/10, optionnel): Extraire les format strings en helper function

### Scope

**In Scope:**
- Extraction de `SLIPPAGE_BUFFER_PCT` comme constante module-level
- Création d'un helper `TradingEvent::with_pair()` pour réduire la duplication
- Optionnel: helper functions `format_percentage()` et `format_price()`
- Tous les tests existants doivent continuer à passer

**Out of Scope:**
- Builder pattern complet (plus invasif, garde API actuelle)
- Changement des signatures publiques des factory methods
- Ajout de nouveaux tests (refactoring pur, couverture existante suffisante)

## Context for Development

### Codebase Patterns

- **Rust idiomatique**: Le code utilise déjà `Self::new()` comme base dans `bot_started()` et `bot_shutdown()` - pattern à étendre
- **Tracing structuré**: Toutes les logs utilisent `tracing::{info, debug, warn, error}` avec des champs structurés
- **Tests inline**: Chaque module contient un bloc `#[cfg(test)] mod tests` en fin de fichier
- **Constantes module-level**: Pas de constantes existantes en haut de `execution.rs` - zone à créer

### Files to Reference

| File | Purpose | Lines concernées |
| ---- | ------- | ---------------- |
| [execution.rs](file:///c:/Users/jules/Documents/bot4/src/core/execution.rs) | Constantes dupliquées | L548, L556 |
| [events.rs](file:///c:/Users/jules/Documents/bot4/src/core/events.rs) | Factory methods répétitives | L187-457 |
| [monitoring.rs](file:///c:/Users/jules/Documents/bot4/src/core/monitoring.rs) | Format strings (optionnel) | L112-113 |
| [runtime.rs](file:///c:/Users/jules/Documents/bot4/src/core/runtime.rs) | Format strings (optionnel) | L91, L134 |

### Investigation Results

**Format strings `format!("{:.4}%", ...)`** - 14 occurrences trouvées:
- `execution.rs`: 6 occurrences (L258, L297, L364, L365, L366, L479)
- `events.rs`: 4 occurrences (L482-485 dans `log_event()`)
- `monitoring.rs`: 2 occurrences (L112-113)
- `runtime.rs`: 2 occurrences (L91, L134)

**Constantes SLIPPAGE_BUFFER**:
- `VEST_SLIPPAGE_BUFFER: f64 = 0.005` - L548
- `PARADEX_SLIPPAGE_BUFFER: f64 = 0.005` - L556
- Valeurs identiques, même usage (protection slippage sur ordres IOC/MARKET)

**Factory methods TradingEvent** - 10 méthodes, 18 champs:
- `new()` - L161-185 - Base pattern, déjà optimisé
- `spread_detected()` - L187-216
- `trade_entry()` - L218-250
- `trade_exit()` - L252-283
- `position_monitoring()` - L285-315
- `order_placed()` - L317-346
- `order_filled()` - L348-377
- `position_closed()` - L379-408
- `bot_started()` - L410-413 - Utilise déjà `Self::new()`
- `slippage_analysis()` - L415-452
- `bot_shutdown()` - L454-457 - Utilise déjà `Self::new()`

### Technical Decisions

- **Option B retenue** pour les factory methods: Garder l'API publique identique, utiliser un helper privé
- **Constante unifiée**: `SLIPPAGE_BUFFER_PCT = 0.005` car les deux valeurs sont identiques
- **Pas de breaking change**: Toutes les signatures publiques restent inchangées
- **Format helpers optionnels**: Scope étape 3 limité à `events.rs` pour éviter trop de fichiers modifiés

## Implementation Plan

### Tasks

**Étape 1: Constantes de slippage** (Risque 2/10)

- [ ] **Task 1.1**: Ajouter constante module-level
  - File: `src/core/execution.rs`
  - Action: Ajouter après les imports (ligne ~24):
    ```rust
    /// Slippage buffer for LIMIT IOC and MARKET orders (0.5% = 50 basis points)
    /// Used as price protection on both Vest and Paradex orders
    const SLIPPAGE_BUFFER_PCT: f64 = 0.005;
    ```
  - Notes: Docstring explique l'usage pour les futurs développeurs

- [ ] **Task 1.2**: Supprimer constantes locales et remplacer usages
  - File: `src/core/execution.rs`
  - Action: Dans `create_orders()` (L546-560):
    - Supprimer `const VEST_SLIPPAGE_BUFFER: f64 = 0.005;` (L548)
    - Supprimer `const PARADEX_SLIPPAGE_BUFFER: f64 = 0.005;` (L556)
    - Remplacer `VEST_SLIPPAGE_BUFFER` par `SLIPPAGE_BUFFER_PCT` (L550-551)
    - Remplacer `PARADEX_SLIPPAGE_BUFFER` par `SLIPPAGE_BUFFER_PCT` (L558-559)

- [ ] **Task 1.3**: Vérification
  - Command: `cargo build && cargo test`
  - Expected: Compilation OK, tous tests passent

---

**Étape 2: Helper method TradingEvent** (Risque 3/10)

- [ ] **Task 2.1**: Créer helper method privé
  - File: `src/core/events.rs`
  - Action: Ajouter après `new()` (ligne ~185):
    ```rust
    /// Helper to create event with pair set (reduces boilerplate in factory methods)
    fn with_pair(event_type: TradingEventType, pair: &str) -> Self {
        let mut event = Self::new(event_type);
        event.pair = Some(pair.to_string());
        event
    }
    ```
  - Notes: Méthode privée (pas de `pub`), signature simple

- [ ] **Task 2.2**: Refactorer `spread_detected()`
  - File: `src/core/events.rs`
  - Action: Remplacer L194-215 par:
    ```rust
    let mut event = Self::with_pair(TradingEventType::SpreadDetected, pair);
    event.exchange = Some("both".to_string());
    event.entry_spread = Some(entry_spread);
    event.spread_threshold = Some(spread_threshold);
    event.direction = Some(direction.to_string());
    event
    ```

- [ ] **Task 2.3**: Refactorer les 7 autres factory methods
  - File: `src/core/events.rs`
  - Pattern pour chaque méthode:
    ```rust
    // Avant: Self { event_type: ..., timestamp_ms: ..., pair: Some(...), ... 18 champs }
    // Après:
    let mut event = Self::with_pair(TradingEventType::XXX, pair);
    event.field1 = Some(value1);
    event.field2 = Some(value2);
    // ... uniquement les champs non-None
    event
    ```
  - Méthodes à transformer:
    - `trade_entry()` → +exchange, entry_spread, threshold, direction, latency, long_exchange, short_exchange
    - `trade_exit()` → +exchange, exit_spread, profit, slippage, timing
    - `position_monitoring()` → +exchange, entry_spread, exit_spread
    - `order_placed()` → +exchange, order_id, direction
    - `order_filled()` → +exchange, order_id, latency
    - `position_closed()` → +exchange, profit, slippage
    - `slippage_analysis()` → +exchange, detection_spread, execution_spread, slippage_bps, timing
  - Notes: `bot_started()` et `bot_shutdown()` restent inchangés (utilisent déjà `Self::new()`)

- [ ] **Task 2.4**: Vérification
  - Command: `cargo build && cargo test`
  - Expected: Compilation OK, tous tests passent

---

**Étape 3: Format helpers** (Risque 1/10, OPTIONNEL)

- [ ] **Task 3.1**: Ajouter helper function
  - File: `src/core/events.rs`
  - Action: Ajouter après `current_timestamp_ms()` (ligne ~466):
    ```rust
    /// Format a percentage value with 4 decimal places
    pub fn format_pct(value: f64) -> String {
        format!("{:.4}%", value)
    }
    ```

- [ ] **Task 3.2**: Remplacer dans `log_event()` uniquement
  - File: `src/core/events.rs`
  - Action: Remplacer L482-485:
    ```rust
    let entry_spread_str = event.entry_spread.map(format_pct);
    let exit_spread_str = event.exit_spread.map(format_pct);
    let threshold_str = event.spread_threshold.map(format_pct);
    let profit_str = event.profit.map(format_pct);
    ```
  - Notes: Ne pas modifier les autres fichiers pour limiter le scope

- [ ] **Task 3.3**: Vérification
  - Command: `cargo build && cargo test`
  - Expected: Compilation OK, tous tests passent

### Acceptance Criteria

- [ ] **AC1**: Given le code modifié, when `cargo build` est exécuté, then la compilation réussit sans erreur ni warning

- [ ] **AC2**: Given le code modifié, when `cargo test` est exécuté, then tous les 14 tests de `events.rs` et les 8 tests de `execution.rs` passent

- [ ] **AC3**: Given `execution.rs`, when on cherche `SLIPPAGE` avec grep, then une seule définition `SLIPPAGE_BUFFER_PCT` existe au niveau module (pas de constantes locales)

- [ ] **AC4**: Given les factory methods de `TradingEvent`, when on compare avant/après, then toutes les signatures publiques sont identiques (pas de breaking change)

- [ ] **AC5**: Given le bot lancé en mode live, when un trade s'exécute, then les logs affichent les mêmes informations qu'avant le refactoring

## Additional Context

### Dependencies

Aucune nouvelle dépendance requise.

### Testing Strategy

**Tests automatisés existants:**
```bash
cargo test -p bot4
```

Les tests pertinents sont dans:
- `src/core/events.rs` (tests des factory methods, L548-703)
- `src/core/execution.rs` (tests de l'executor, L606-921)

**Vérification manuelle:**
- Lancer le bot en mode dry-run et vérifier que les logs sont identiques

### Notes

- Ce refactoring est **purement cosmétique** - aucun changement de comportement
- Procéder étape par étape permet de rollback facilement si problème
- Les tests existants sont suffisants car on ne change pas la logique
