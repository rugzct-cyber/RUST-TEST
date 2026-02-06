---
title: 'Fix Vest Leverage Setup in Bot Runtime'
slug: 'vest-leverage-fix'
created: '2026-02-06T17:03:18+01:00'
status: 'ready-for-dev'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, tokio, vest-api, paradex-api, eip-712-signing]
files_to_modify: ['src/main.rs']
code_patterns: ['async initialization', 'adapter configuration', 'match error handling']
test_patterns: ['cargo run --bin test_order', 'cargo run --bin delta_neutral_cycle']
---

# Tech-Spec: Fix Vest Leverage Setup in Bot Runtime

**Created:** 2026-02-06T17:03:18+01:00

## Overview

### Problem Statement

Le bot ne peut pas fermer les positions sur Vest. La cause racine est que le runtime principal (`main.rs`) ne configure jamais le leverage sur les exchanges, contrairement aux binaires de test qui appellent `set_leverage()` correctement. Vest requiert un leverage configuré avant de pouvoir placer des ordres, et potentiellement pour les ordres `reduce_only` utilisés pour fermer les positions.

### Solution

Ajouter un appel à `set_leverage()` sur les deux adapters d'exécution (Vest et Paradex) dans `main.rs`, juste après leur connexion et avant la création du `DeltaNeutralExecutor`.

### Scope

**In Scope:**
- Ajouter `set_leverage()` pour les execution adapters dans `main.rs`
- Utiliser la valeur `bot.leverage` du fichier `config.yaml`
- Log structuré pour confirm le setup

**Out of Scope:**
- Changements à l'API de leverage
- Modification des adapters eux-mêmes
- Test de la fermeture de position (sera fait manuellement après le fix)

## Context for Development

### Codebase Patterns

- **Vest set_leverage:** `adapter.set_leverage(symbol: &str, leverage: u32) -> ExchangeResult<u32>` (ligne 914 adapter.rs)
- **Paradex set_leverage:** Même signature (ligne 767 adapter.rs)
- **Pattern test_order.rs:** ligne 132 appelle `adapter.set_leverage(VEST_PAIR, TARGET_LEVERAGE).await`
- **Pattern delta_neutral_cycle.rs:** lignes 57-58 pour les deux exchanges
- **Error handling:** `match` avec `Ok(lev) =>` info log et `Err(e) =>` warn log (continue)

### Files to Reference

| File | Purpose |
| ---- | ------- |
| [main.rs:L260-261](file:///c:/Users/jules/Documents/bot4/bot/src/main.rs#L260-L261) | **Point d'insertion exact** - après les deux `.connect()` |
| [main.rs:L262](file:///c:/Users/jules/Documents/bot4/bot/src/main.rs#L262) | Avant DeltaNeutralExecutor::new() |
| [adapter.rs:L914-977](file:///c:/Users/jules/Documents/bot4/bot/src/adapters/vest/adapter.rs#L914-L977) | Implémentation Vest set_leverage (EIP-712 signed) |
| [adapter.rs:L767](file:///c:/Users/jules/Documents/bot4/bot/src/adapters/paradex/adapter.rs#L767) | Implémentation Paradex set_leverage |
| [config.yaml](file:///c:/Users/jules/Documents/bot4/bot/config.yaml) | Source du leverage (bot.leverage: u8) |

### Technical Decisions

1. **Placement:** Après `execution_vest.connect().await` et `execution_paradex.connect().await`, avant `DeltaNeutralExecutor::new()`
2. **Error handling:** Continue on error (comme dans les tests) - log un warning mais ne pas crash. Vest peut avoir le leverage déjà set.
3. **Leverage type:** Config `leverage` est `u8`, cast vers `u32` pour l'API

## Implementation Plan

### Tasks

#### Task 1: Add leverage setup after execution adapter connection [x]

**File:** `src/main.rs`
**Location:** After line 260 (after execution_paradex.connect())
**Action:** Insert leverage setup code block

```rust
// Set leverage on both execution adapters (from config)
let target_leverage = bot.leverage as u32;
info!(event_type = "LEVERAGE_SETUP", leverage = %format!("{}x", target_leverage), "Setting leverage on execution adapters");

match execution_vest.set_leverage(&vest_symbol, target_leverage).await {
    Ok(lev) => info!(event_type = "LEVERAGE_SETUP", exchange = "vest", leverage = lev, "Leverage configured"),
    Err(e) => warn!(event_type = "LEVERAGE_SETUP", exchange = "vest", error = %e, "Failed to set leverage (continuing)"),
}

match execution_paradex.set_leverage(&paradex_symbol, target_leverage).await {
    Ok(lev) => info!(event_type = "LEVERAGE_SETUP", exchange = "paradex", leverage = lev, "Leverage configured"),
    Err(e) => warn!(event_type = "LEVERAGE_SETUP", exchange = "paradex", error = %e, "Failed to set leverage (continuing)"),
}
```

### Acceptance Criteria

**AC1: Leverage is set on startup**
- Given: Bot starts with `leverage: 5` in config.yaml
- When: Bot connects to exchanges
- Then: Logs show `LEVERAGE_SETUP` events for vest et paradex

**AC2: Graceful error handling**
- Given: Leverage API fails (rate limit, already set, etc.)
- When: `set_leverage` returns error
- Then: Bot logs warning but continues (does not crash)

**AC3: Correct leverage value**
- Given: `config.yaml` has `leverage: 5`
- When: Leverage is set on Vest
- Then: Vest account shows 5x leverage for BTC-PERP

## Additional Context

### Dependencies

Aucune nouvelle dépendance requise.

### Testing Strategy

**Test manuel:**
1. `cargo build` - vérifier compilation
2. `cargo run --bin test_order` - ce binary teste déjà le leverage setup sur Vest
3. Start le bot et vérifier les logs pour `LEVERAGE_SETUP` events
4. (Manuel) Tenter ouvrir/fermer une position pour valider le fix complet

### Notes

- Cette spec est un fix minimal pour débloquer les tests
- Investigation supplémentaire après exécution pour identifier la cause exacte du problème de fermeture
