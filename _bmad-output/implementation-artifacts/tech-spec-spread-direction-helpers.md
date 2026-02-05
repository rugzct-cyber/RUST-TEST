---
title: 'SpreadDirection Helper Methods'
slug: 'spread-direction-helpers'
created: '2026-02-04'
status: 'ready-for-dev'
stepsCompleted: [1, 2, 3]
tech_stack: [Rust]
files_to_modify: [spread.rs, execution.rs]
code_patterns: [SRP, DRY]
test_patterns: [unit-test]
---

# Tech-Spec: SpreadDirection Helper Methods

**Created:** 2026-02-04 | **Impact:** MOYEN | **Risque:** 2/10 | **Effort:** FAIBLE

## Overview

### Problem Statement

Calcul du spread basé sur direction répété **5 fois** dans `execution.rs`:

| Ligne | Pattern | Usage |
|-------|---------|-------|
| L288-291 | `match direction → (long_ex, short_ex)` | Determine exchanges |
| L372-375 | `match direction → u8` | Atomic storage |
| L419-432 | `match direction → spread calc` | Captured spread |
| L502-505 | `match direction → OrderSide` | Close order sides |
| L563-566 | `match direction → (status, exchange)` | Result mapping |

Cette duplication viole DRY et complique la maintenance.

### Solution

Ajouter 5 méthodes helper à `SpreadDirection` enum dans `spread.rs`:

```rust
impl SpreadDirection {
    pub fn to_exchanges(&self) -> (&'static str, &'static str)
    pub fn to_u8(&self) -> u8
    pub fn from_u8(value: u8) -> Option<Self>
    pub fn to_close_sides(&self) -> (OrderSide, OrderSide)
    pub fn calculate_captured_spread(&self, vest_price: f64, paradex_price: f64) -> f64
}
```

### Scope

**In Scope:**
- Ajouter 5 méthodes helper sur `SpreadDirection`
- Refactorer les 5 sites de duplication dans `execution.rs`
- Tests unitaires pour chaque helper

**Out of Scope:**
- Modifications aux binaires de test
- Refactoring d'autres patterns

## Context for Development

### Codebase Patterns

- **SRP Pattern**: Logique direction centralisée dans `SpreadDirection`
- **Safe Return Types**: `from_u8()` retourne `Option<Self>` (Red Team Pattern)
- **Re-export Pattern**: Ajouter export dans `src/core/mod.rs` si nécessaire

### Files to Reference

| File | Purpose |
| ---- | ------- |
| [spread.rs](file:///c:/Users/jules/Documents/bot4/src/core/spread.rs) | Target: SpreadDirection enum (L20-25) |
| [execution.rs](file:///c:/Users/jules/Documents/bot4/src/core/execution.rs) | Consumer: 5 refactoring sites |

### Technical Decisions

1. **OrderSide Import**: `to_close_sides()` nécessite import de `OrderSide` dans spread.rs
2. **Exchange Names Hardcoded**: `"vest"` et `"paradex"` codés en dur (cohérent avec codebase)
3. **Calculation Formula**: Identique à formule existante dans `verify_positions()`

## Implementation Plan

### Tasks

#### Task 1: Ajouter helpers dans `spread.rs`

**File:** `src/core/spread.rs`

Ajouter import au début du fichier:
```rust
use crate::adapters::types::OrderSide;
```

Ajouter bloc `impl SpreadDirection` après la définition de l'enum (après L25):

```rust
impl SpreadDirection {
    /// Returns (long_exchange, short_exchange) for Vest/Paradex setup
    pub fn to_exchanges(&self) -> (&'static str, &'static str) {
        match self {
            SpreadDirection::AOverB => ("vest", "paradex"),
            SpreadDirection::BOverA => ("paradex", "vest"),
        }
    }
    
    /// Convert to atomic storage value (1=AOverB, 2=BOverA)
    pub fn to_u8(&self) -> u8 {
        match self {
            SpreadDirection::AOverB => 1,
            SpreadDirection::BOverA => 2,
        }
    }
    
    /// Create from atomic storage value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(SpreadDirection::AOverB),
            2 => Some(SpreadDirection::BOverA),
            _ => None,
        }
    }
    
    /// Get close order sides (to reverse the position)
    pub fn to_close_sides(&self) -> (OrderSide, OrderSide) {
        match self {
            SpreadDirection::AOverB => (OrderSide::Sell, OrderSide::Buy),
            SpreadDirection::BOverA => (OrderSide::Buy, OrderSide::Sell),
        }
    }
    
    /// Calculate captured spread from entry prices
    pub fn calculate_captured_spread(&self, vest_price: f64, paradex_price: f64) -> f64 {
        match self {
            SpreadDirection::AOverB => {
                if vest_price > 0.0 {
                    ((paradex_price - vest_price) / vest_price) * 100.0
                } else { 0.0 }
            }
            SpreadDirection::BOverA => {
                if paradex_price > 0.0 {
                    ((vest_price - paradex_price) / paradex_price) * 100.0
                } else { 0.0 }
            }
        }
    }
}
```

---

#### Task 2: Refactorer `execution.rs` - Site 1 (L288-291)

**Before:**
```rust
let (long_exchange, short_exchange) = match opportunity.direction {
    SpreadDirection::AOverB => ("vest", "paradex"),
    SpreadDirection::BOverA => ("paradex", "vest"),
};
```

**After:**
```rust
let (long_exchange, short_exchange) = opportunity.direction.to_exchanges();
```

---

#### Task 3: Refactorer `execution.rs` - Site 2 (L372-375)

**Before:**
```rust
let dir_value = match opportunity.direction {
    SpreadDirection::AOverB => 1u8,
    SpreadDirection::BOverA => 2u8,
};
```

**After:**
```rust
let dir_value = opportunity.direction.to_u8();
```

---

#### Task 4: Refactorer `execution.rs` - Site 3 (L419-432)

**Before:**
```rust
let captured_spread = match entry_direction {
    Some(SpreadDirection::AOverB) => {
        if vest_price > 0.0 {
            ((paradex_price - vest_price) / vest_price) * 100.0
        } else { 0.0 }
    }
    Some(SpreadDirection::BOverA) => {
        if paradex_price > 0.0 {
            ((vest_price - paradex_price) / paradex_price) * 100.0
        } else { 0.0 }
    }
    None => 0.0,
};
```

**After:**
```rust
let captured_spread = entry_direction
    .map(|dir| dir.calculate_captured_spread(vest_price, paradex_price))
    .unwrap_or(0.0);
```

---

#### Task 5: Refactorer `execution.rs` - Site 4 (L502-505)

**Before:**
```rust
let (vest_side, paradex_side) = match entry_dir {
    SpreadDirection::AOverB => (OrderSide::Sell, OrderSide::Buy),
    SpreadDirection::BOverA => (OrderSide::Buy, OrderSide::Sell),
};
```

**After:**
```rust
let (vest_side, paradex_side) = entry_dir.to_close_sides();
```

---

#### Task 6: Refactorer `execution.rs` - Site 5 (L563-566)

**Before:**
```rust
let (long_status, short_status, long_exchange, short_exchange) = match entry_dir {
    SpreadDirection::AOverB => (vest_status, paradex_status, "vest", "paradex"),
    SpreadDirection::BOverA => (paradex_status.clone(), vest_status.clone(), "paradex", "vest"),
};
```

**After:**
```rust
let (long_exchange, short_exchange) = entry_dir.to_exchanges();
let (long_status, short_status) = if matches!(entry_dir, SpreadDirection::AOverB) {
    (vest_status, paradex_status)
} else {
    (paradex_status.clone(), vest_status.clone())
};
```

---

#### Task 7: Refactorer `get_entry_direction()` (L476-481)

**Before:**
```rust
pub fn get_entry_direction(&self) -> Option<SpreadDirection> {
    match self.entry_direction.load(Ordering::SeqCst) {
        1 => Some(SpreadDirection::AOverB),
        2 => Some(SpreadDirection::BOverA),
        _ => None,
    }
}
```

**After:**
```rust
pub fn get_entry_direction(&self) -> Option<SpreadDirection> {
    SpreadDirection::from_u8(self.entry_direction.load(Ordering::SeqCst))
}
```

---

#### Task 8: Tests unitaires dans `spread.rs`

Ajouter dans le bloc `#[cfg(test)] mod tests`:

```rust
// =========================================================================
// SpreadDirection Helper Tests
// =========================================================================

#[test]
fn test_spread_direction_to_exchanges() {
    assert_eq!(SpreadDirection::AOverB.to_exchanges(), ("vest", "paradex"));
    assert_eq!(SpreadDirection::BOverA.to_exchanges(), ("paradex", "vest"));
}

#[test]
fn test_spread_direction_to_u8() {
    assert_eq!(SpreadDirection::AOverB.to_u8(), 1);
    assert_eq!(SpreadDirection::BOverA.to_u8(), 2);
}

#[test]
fn test_spread_direction_from_u8() {
    assert_eq!(SpreadDirection::from_u8(1), Some(SpreadDirection::AOverB));
    assert_eq!(SpreadDirection::from_u8(2), Some(SpreadDirection::BOverA));
    assert_eq!(SpreadDirection::from_u8(0), None);
    assert_eq!(SpreadDirection::from_u8(3), None);
}

#[test]
fn test_spread_direction_to_close_sides() {
    use crate::adapters::types::OrderSide;
    assert_eq!(SpreadDirection::AOverB.to_close_sides(), (OrderSide::Sell, OrderSide::Buy));
    assert_eq!(SpreadDirection::BOverA.to_close_sides(), (OrderSide::Buy, OrderSide::Sell));
}

#[test]
fn test_spread_direction_calculate_captured_spread_a_over_b() {
    // Long Vest at 42000, Short Paradex at 42100
    // Spread = (42100 - 42000) / 42000 * 100 = 0.238%
    let spread = SpreadDirection::AOverB.calculate_captured_spread(42000.0, 42100.0);
    assert!((spread - 0.238).abs() < 0.01);
}

#[test]
fn test_spread_direction_calculate_captured_spread_b_over_a() {
    // Long Paradex at 42000, Short Vest at 42100
    // Spread = (42100 - 42000) / 42000 * 100 = 0.238%
    let spread = SpreadDirection::BOverA.calculate_captured_spread(42100.0, 42000.0);
    assert!((spread - 0.238).abs() < 0.01);
}

#[test]
fn test_spread_direction_calculate_captured_spread_zero_price() {
    assert_eq!(SpreadDirection::AOverB.calculate_captured_spread(0.0, 42100.0), 0.0);
    assert_eq!(SpreadDirection::BOverA.calculate_captured_spread(42100.0, 0.0), 0.0);
}

#[test]
fn test_spread_direction_roundtrip() {
    // Verify to_u8 and from_u8 are inverses
    for dir in [SpreadDirection::AOverB, SpreadDirection::BOverA] {
        assert_eq!(SpreadDirection::from_u8(dir.to_u8()), Some(dir));
    }
}
```

### Acceptance Criteria

**AC1: Helpers compilent et passent les tests**
- **Given** les nouvelles méthodes dans `SpreadDirection`
- **When** `cargo test -p bot4 spread::tests`
- **Then** tous les tests passent sans erreur

**AC2: Refactoring execution.rs préserve le comportement**
- **Given** les sites refactorisés dans `execution.rs`
- **When** `cargo test -p bot4 execution::tests`
- **Then** tous les tests existants passent

**AC3: Compilation globale**
- **Given** tous les changements appliqués
- **When** `cargo build --release`
- **Then** aucune erreur ni warning

## Additional Context

### Dependencies

- `OrderSide` doit être importé dans `spread.rs`
- Aucune nouvelle dépendance externe

### Verification Plan

```bash
# 1. Run all spread tests
cargo test -p bot4 spread::tests

# 2. Run execution tests
cargo test -p bot4 execution::tests

# 3. Full build check
cargo build --release

# 4. Clippy check
cargo clippy -p bot4 -- -D warnings
```

### Bénéfices

- ✅ ~30 lignes éliminées
- ✅ Logique centralisée = facile à modifier
- ✅ `get_entry_direction()` simplifié
- ✅ Tests unitaires dédiés pour chaque helper
