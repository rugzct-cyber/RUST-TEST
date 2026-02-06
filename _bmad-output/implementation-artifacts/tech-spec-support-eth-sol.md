---
title: 'Support ETH et SOL'
slug: 'support-eth-sol'
created: '2026-02-06T18:01:41+01:00'
status: 'ready-for-testing'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, serde, tokio]
files_to_modify:
  - bot/config.yaml
code_patterns:
  - TradingPair enum already supports ETH-PERP and SOL-PERP
  - Symbol mapping: Vest uses pair directly, Paradex adds -USD- prefix
  - Vest API confirms: "All crypto perpetual symbols are {COIN}-PERP, e.g. BTC-PERP, SOL-PERP"
  - Paradex tests confirm ETH-USD-PERP format support
test_patterns:
  - Existing unit tests use BTC-PERP, ETH-PERP in adapters/types.rs
  - Paradex signing tests use ETH-USD-PERP
investigation_results:
  vest_confirmed: true
  paradex_confirmed: true
  code_changes_needed: false
---

# Tech-Spec: Support ETH et SOL

**Created:** 2026-02-06

## Overview

### Problem Statement

Le bot ne traite actuellement que BTC-PERP. L'utilisateur veut pouvoir configurer et tester ETH-PERP ou SOL-PERP avec les mêmes fonctionnalités d'arbitrage entre Vest et Paradex.

### Solution

**Aucun changement de code nécessaire.** Le codebase supporte déjà ces paires via l'enum `TradingPair` et le mapping dynamique des symboles. Il suffit de modifier le `config.yaml` pour utiliser ETH ou SOL.

### Scope

**In Scope:**
- Configurations d'exemple pour ETH et SOL dans config.yaml
- Documentation des position_size recommandées par coin
- Vérification fonctionnelle sur les 2 exchanges

**Out of Scope:**
- Mode multi-pair simultané
- Ajout de nouveaux DEX
- Nouveaux coins autres que ETH et SOL

## Context for Development

### Codebase Patterns

1. **TradingPair Enum** (`config/types.rs` L24-32): Supporte `BtcPerp`, `EthPerp`, `SolPerp`
2. **Symbol Mapping** (`main.rs` L188-191):
   - Vest: `bot.pair.to_string()` → `ETH-PERP`
   - Paradex: `{COIN}-USD-PERP` → `ETH-USD-PERP`
3. **Single-bot MVP**: `config.bots[0]` - un seul bot actif

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `bot/config.yaml` | Configuration actuelle (BTC uniquement) |
| `bot/src/config/types.rs` | Enum TradingPair (lignes 24-32) |
| `bot/src/main.rs` | Mapping symboles (lignes 188-191) |

### Technical Decisions

- **Pas de code à modifier** : l'architecture existante est déjà générique
- **Position sizes** : valeurs adaptées à chaque coin basées sur le prix ~100x différent

## Implementation Plan

### Tasks

- [x] **Task 1:** Ajouter configurations ETH et SOL dans config.yaml
  - File: `bot/config.yaml`
  - Action: Ajouter 2 bots commentés (eth_vest_paradex, sol_vest_paradex) avec position_size adaptés
  - Notes: Utiliser position_size 0.1 pour ETH, 1.0 pour SOL

- [ ] **Task 2:** Test fonctionnel ETH
  - File: `bot/config.yaml`
  - Action: Décommenter le bot ETH, commenter le bot BTC, lancer le bot
  - Notes: Vérifier logs de connexion + spreads dans TUI

- [ ] **Task 3:** Test fonctionnel SOL
  - File: `bot/config.yaml`
  - Action: Utiliser le bot SOL en premier, lancer le bot
  - Notes: Vérifier logs de connexion + spreads dans TUI

### Acceptance Criteria

- [ ] **AC1:** Given config.yaml avec `pair: ETH-PERP` en premier bot, when le bot démarre, then il se connecte à Vest (`ETH-PERP`) et Paradex (`ETH-USD-PERP`) et affiche les spreads dans le TUI

- [ ] **AC2:** Given config.yaml avec `pair: SOL-PERP` en premier bot, when le bot démarre, then il se connecte à Vest (`SOL-PERP`) et Paradex (`SOL-USD-PERP`) et affiche les spreads dans le TUI

- [ ] **AC3:** Given config.yaml avec `pair: BTC-PERP` (inchangé), when le bot démarre, then le comportement est identique à avant (pas de régression)

## Additional Context

### Dependencies

- Aucune nouvelle dépendance Cargo
- Credentials .env existants fonctionnent pour tous les coins

### Testing Strategy

**Tests manuels uniquement** (pas de tests automatisés nécessaires car aucun code modifié) :

1. **Test ETH:**
   ```bash
   # Modifier config.yaml: pair: ETH-PERP
   cd bot && $env:LOG_FORMAT="tui"; cargo run --release --bin hft_bot
   # Vérifier: header TUI affiche "ETH-PERP", spreads s'affichent
   ```

2. **Test SOL:**
   ```bash
   # Modifier config.yaml: pair: SOL-PERP  
   cd bot && $env:LOG_FORMAT="tui"; cargo run --release --bin hft_bot
   # Vérifier: header TUI affiche "SOL-PERP", spreads s'affichent
   ```

3. **Test régression BTC:**
   ```bash
   # Remettre config.yaml: pair: BTC-PERP
   cd bot && $env:LOG_FORMAT="tui"; cargo run --release --bin hft_bot
   # Vérifier: comportement identique à avant
   ```

### Notes

- Position size recommandée ETH: 0.1 (prix ~$2500 → ~$250 de position)
- Position size recommandée SOL: 1.0 (prix ~$100 → ~$100 de position)
- Les spreads entry/exit peuvent être ajustés selon la volatilité de chaque coin
