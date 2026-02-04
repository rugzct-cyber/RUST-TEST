---
title: 'OrderRequest Builder Pattern'
slug: 'orderrequest-builder-pattern'
created: '2026-02-04T19:07:35+01:00'
status: 'completed'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, tokio, tracing]
files_to_modify:
  - src/adapters/types.rs
  - src/core/execution.rs
code_patterns:
  - builder-pattern
  - method-chaining
  - explicit-re-exports
test_patterns:
  - inline-unit-tests
  - cargo-test
---

# Tech-Spec: OrderRequest Builder Pattern

**Created:** 2026-02-04T19:07:35+01:00

## Overview

### Problem Statement

Création manuelle d'`OrderRequest` répétée 4+ fois avec beaucoup de boilerplate (~22 lignes pour 2 ordres). Chaque création nécessite de spécifier tous les 8 champs même quand plusieurs ont des valeurs par défaut constantes.

### Solution

Implémenter un `OrderBuilder` avec defaults intelligents et méthodes chainables dans `adapters/types.rs`, directement après la définition de `OrderRequest`. Le builder suit le pattern standard Rust (reqwest, tokio) avec méthodes fluides.

### Scope

**In Scope:**
- Création du struct `OrderBuilder` dans `adapters/types.rs`
- Refactoring des 4 occurrences dans `execution.rs`:
  - 2 dans `close_position()` (lignes 511-531)
  - 2 dans `create_orders()` (lignes 637-658)

**Out of Scope:**
- Les 19 occurrences dans `src/bin/*` (Phase 2 optionnelle)
- Modification des factory methods existantes (`limit()`, `ioc_limit()`)

## Context for Development

### Codebase Patterns

- **Architecture**: Explicit re-export pattern via `adapters/mod.rs`
- **Tests**: Inline `#[cfg(test)] mod tests` dans chaque fichier source
- **Conventions**: Types publics avec `pub`, méthodes chainables retournant `Self`
- `OrderRequest` défini ligne 157 de `types.rs`, suivi d'un `impl` (lignes 176-228)
- Factory methods existantes (`limit()`, `ioc_limit()`) non utilisées dans le code actuel

### Files to Reference

| File | Purpose |
| ---- | ------- |
| [types.rs](file:///c:/Users/jules/Documents/bot4/src/adapters/types.rs#L157-L228) | `OrderRequest` struct + impl existant |
| [mod.rs](file:///c:/Users/jules/Documents/bot4/src/adapters/mod.rs#L14-L18) | Re-exports publics - doit exporter `OrderBuilder` |
| [execution.rs](file:///c:/Users/jules/Documents/bot4/src/core/execution.rs#L511-L661) | 4 occurrences à refactorer |

### Technical Decisions

1. **Builder séparé vs factory methods**: Builder séparé pour flexibilité et lisibilité
2. **Defaults** (basés sur analyse 100% du code existant):
   - `time_in_force`: `TimeInForce::Ioc`
   - `order_type`: `OrderType::Limit`
   - `reduce_only`: `false`
   - `client_order_id`: `String::new()` (doit être set)
3. **Visibilité**: `pub struct OrderBuilder` + re-export dans `mod.rs`
4. **🔴 Red Team Hardening** (analyse adversariale):
   - `build()` retourne `Result<OrderRequest, &'static str>` (pas `OrderRequest`)
   - Validation obligatoire: `client_order_id` non vide
   - Appel à `OrderRequest::validate()` dans `build()`

### Red Team Analysis (Applied)

| Vulnérabilité | Sévérité | Contre-mesure |
|---------------|----------|---------------|
| Oubli de `client_order_id` | 🔴 CRITICAL | Erreur si vide dans `build()` |
| `build()` sans validation | 🟠 HIGH | `build() -> Result<...>` avec validation |
| Double default `order_type` | 🟡 MEDIUM | Documentation claire |

## Implementation Plan

### Tasks

- [x] **Task 1: Créer `OrderBuilder` struct dans `types.rs`**
  - File: `src/adapters/types.rs`
  - Action: Ajouter après ligne 228 (fin de `impl OrderRequest`):
    ```rust
    /// Builder for OrderRequest with sensible defaults for HFT
    pub struct OrderBuilder {
        symbol: String,
        side: OrderSide,
        quantity: f64,
        client_order_id: String,
        order_type: OrderType,
        price: Option<f64>,
        time_in_force: TimeInForce,
        reduce_only: bool,
    }
    ```
  - Notes: Tous les champs sont privés, modifiés uniquement via méthodes

- [x] **Task 2: Implémenter `impl OrderBuilder` avec méthodes chainables**
  - File: `src/adapters/types.rs`
  - Action: Ajouter après le struct:
    - `new(symbol, side, quantity)` → constructeur avec defaults
    - `client_order_id(id)` → setter obligatoire
    - `market()` → switch vers `OrderType::Market`
    - `limit(price)` → switch vers `OrderType::Limit` avec price
    - `reduce_only()` → active reduce_only
    - `build() -> Result<OrderRequest, &'static str>` → construit avec validation
  - Notes: Defaults = `Ioc`, `Limit`, `reduce_only: false`

- [x] **Task 3: Ajouter re-export dans `mod.rs`**
  - File: `src/adapters/mod.rs`
  - Action: Modifier ligne 14-17 pour ajouter `OrderBuilder`:
    ```rust
    pub use types::{
        Orderbook, OrderbookLevel, OrderbookUpdate,
        OrderRequest, OrderResponse, OrderSide, OrderStatus, OrderType, TimeInForce,
        PositionInfo, OrderBuilder,
    };
    ```
  - Notes: Permet l'import depuis `crate::adapters::OrderBuilder`

- [x] **Task 4: Refactorer `close_position()` dans `execution.rs`**
  - File: `src/core/execution.rs`
  - Action: Remplacer lignes 511-531 (2 créations `OrderRequest`) par:
    ```rust
    let vest_order = OrderBuilder::new(&self.vest_symbol, vest_side, self.default_quantity)
        .client_order_id(format!("close-vest-{}", timestamp))
        .market()
        .reduce_only()
        .build()
        .expect("close_position order should be valid");

    let paradex_order = OrderBuilder::new(&self.paradex_symbol, paradex_side, self.default_quantity)
        .client_order_id(format!("close-paradex-{}", timestamp))
        .market()
        .reduce_only()
        .build()
        .expect("close_position order should be valid");
    ```
  - Notes: `.expect()` acceptable car les paramètres sont contrôlés

- [x] **Task 5: Refactorer `create_orders()` dans `execution.rs`**
  - File: `src/core/execution.rs`
  - Action: Remplacer lignes 637-658 (2 créations `OrderRequest`) par:
    ```rust
    let vest_order = OrderBuilder::new(&self.vest_symbol, vest_side, quantity)
        .client_order_id(vest_order_id)
        .market()
        .price(vest_price)  // Vest slippage protection (keeps Market type)
        .build()
        .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;

    let paradex_order = OrderBuilder::new(&self.paradex_symbol, paradex_side, quantity)
        .client_order_id(paradex_order_id)
        .limit(paradex_price)
        .build()
        .map_err(|e| ExchangeError::InvalidOrder(e.to_string()))?;
    ```
  - Notes: Vest utilise `Market` + `price()` pour slippage protection

- [x] **Task 6: Ajouter import `OrderBuilder` dans `execution.rs`**
  - File: `src/core/execution.rs`
  - Action: Ajouter à l'import existant de `crate::adapters`:
    ```rust
    use crate::adapters::{..., OrderBuilder};
    ```
  - Notes: Vérifier que l'import compile

- [x] **Task 7: Ajouter tests unitaires pour `OrderBuilder`**
  - File: `src/adapters/types.rs`
  - Action: Ajouter dans le module `#[cfg(test)] mod tests`:
    ```rust
    #[test]
    fn test_order_builder_happy_path() { ... }
    
    #[test]
    fn test_order_builder_missing_client_order_id() { ... }
    
    #[test]
    fn test_order_builder_limit_without_price() { ... }
    
    #[test]
    fn test_order_builder_market_order() { ... }
    
    #[test]
    fn test_order_builder_reduce_only() { ... }
    ```
  - Notes: 5 tests minimum couvrant happy path + erreurs

- [x] **Task 8: Vérification finale**
  - Action: Exécuter `cargo build` et `cargo test`
  - Notes: Tous les tests doivent passer, pas de warnings

### Acceptance Criteria

- [x] **AC1**: Given un `OrderBuilder` avec tous les champs valides, when `build()` est appelé, then `Ok(OrderRequest)` est retourné avec les valeurs correctes
- [x] **AC2**: Given un `OrderBuilder` avec `client_order_id` vide, when `build()` est appelé, then `Err("client_order_id is required")` est retourné
- [x] **AC3**: Given un `OrderBuilder` avec `OrderType::Limit` et `price: None`, when `build()` est appelé, then `Err("Limit orders require a price")` est retourné
- [x] **AC4**: Given `close_position()` appelée, when les ordres sont créés via `OrderBuilder`, then le comportement est identique à l'implémentation actuelle
- [x] **AC5**: Given `create_orders()` appelée, when les ordres sont créés via `OrderBuilder`, then le comportement est identique à l'implémentation actuelle
- [x] **AC6**: Given le code refactoré, when `cargo build` est exécuté, then la compilation réussit sans warnings
- [x] **AC7**: Given le code refactoré, when `cargo test` est exécuté, then tous les tests passent (existants + nouveaux)

## Additional Context

### Dependencies

Aucune nouvelle dépendance requise.

### Testing Strategy

**Tests automatisés:**
```bash
# Vérification compilation
cargo build

# Exécution tests unitaires
cargo test

# Tests spécifiques OrderBuilder
cargo test order_builder
```

**Tests unitaires à ajouter dans `types.rs`:**
- `test_order_builder_happy_path` - Création complète → `Ok`
- `test_order_builder_missing_client_order_id` - ID vide → `Err`
- `test_order_builder_limit_without_price` - Limit sans prix → `Err`
- `test_order_builder_market_order` - `.market()` fonctionne
- `test_order_builder_reduce_only` - `.reduce_only()` fonctionne

### Notes

- Risque estimé: 3/10 (refactoring simple, pas de changement de comportement)
- Bénéfice: ~55% réduction de code pour création d'ordres
- 🔒 Hardened: Validation obligatoire prévient les erreurs runtime
- ⚠️ `.expect()` utilisé dans le code core car les paramètres sont contrôlés


