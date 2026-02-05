---
title: 'Reader Task Duplication Refactoring'
slug: 'reader-task-duplication'
created: '2026-02-05T01:25:00+01:00'
status: 'in-progress'
stepsCompleted: [1]
tech_stack: ['rust', 'tokio', 'futures-util', 'serde']
files_to_modify:
  - src/adapters/paradex/adapter.rs
  - src/adapters/vest/adapter.rs
  - src/adapters/shared/reader.rs (NEW)
code_patterns:
  - trait-based-abstraction
  - async-stream-processing
test_patterns:
  - unit-tests
  - cargo-build-verification
---

# Tech-Spec: Reader Task Duplication Refactoring

**Created:** 2026-02-05T01:25:00+01:00

## Overview

### Problem Statement

Les deux adapters (Paradex et Vest) implémentent des `message_reader_loop` quasi-similaires pour traiter les messages WebSocket et mettre à jour les orderbooks partagés. Cette duplication cause:
- Code répété difficile à maintenir
- Risque d'incohérence lors de corrections de bugs
- Violation du principe DRY

### Solution

Créer un trait `MessageParser<M>` et une fonction générique `message_reader_loop<M, P>` dans un nouveau module `src/adapters/shared/reader.rs` pour factoriser la logique commune.

### Scope

**In Scope:**
- Extraction du pattern commun de lecture WebSocket
- Création du trait `MessageParser` avec méthodes de parsing
- Refactoring des deux adapters pour utiliser la fonction générique

**Out of Scope:**
- Modification de la logique de heartbeat (reste spécifique)
- Refactoring des méthodes `split_and_spawn_reader`
- Changement du format des messages WebSocket

## Context for Development

### Codebase Patterns

- Pattern existant: `trait ExchangeAdapter` pour abstraction des exchanges
- Types de messages: `ParadexWsMessage` (3 variants), `VestWsMessage` (3 variants)
- Tous utilisent `serde::Deserialize` + `serde(untagged)`

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `src/adapters/paradex/adapter.rs:434-571` | Paradex message_reader_loop |
| `src/adapters/vest/adapter.rs:685-787` | Vest message_reader_loop |
| `src/adapters/paradex/types.rs` | ParadexWsMessage enum |
| `src/adapters/vest/types.rs` | VestWsMessage enum |
| `src/adapters/types.rs` | SharedOrderbooks type |

### Technical Decisions

**À CONFIRMER avec l'utilisateur avant implémentation.**

## Implementation Plan

### Tasks

*À définir après confirmation du scope*

### Acceptance Criteria

*À définir après confirmation du scope*

## Additional Context

### Dependencies

- `futures-util` (StreamExt pour next())
- `tokio-tungstenite` (Message type)
- `serde` (Désérialisation des messages)

### Testing Strategy

*À définir*

### Notes

**Analyse de duplication (investigation réelle):**

| Aspect | Paradex | Vest | Similarité |
|--------|---------|------|------------|
| Lignes totales | ~137 (L434-571) | ~102 (L685-787) | - |
| Loop structure | `while let Some` | `while let Some` | 100% |
| Health timestamp | `last_data.store()` | `last_data.store()` | 100% |
| Parse message | `serde_json::from_str` | `serde_json::from_str` | 100% |
| Orderbook update | `shared_orderbooks.write()` | `shared_orderbooks.write()` | 100% |
| Ping/Pong | Natif (tokio-tungstenite) | Custom PING/PONG + `last_pong` | **0%** |
| Order channel | Handler inline (~25 lignes) | N/A | **0%** |
| Binary fallback | N/A | ~20 lignes | **0%** |
| Close handling | Log + break | Log + break | 100% |
| Error handling | Log + break | Log + break | 100% |

**Estimation révisée:** ~50-60% de similarité (pas 400 lignes identiques)
