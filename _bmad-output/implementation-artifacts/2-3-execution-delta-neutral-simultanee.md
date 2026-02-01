# Story 2.3: Exécution Delta-Neutral Simultanée

Status: review

<!-- Note: FR7 implementation. Uses existing place_order() from Stories 2-1 and 2-2. Key constraint: NFR2 <500ms detection-to-order latency. -->

## Story

As a **opérateur**,
I want que le bot exécute simultanément un ordre long et un ordre short,
So that ma position soit delta-neutral dès l'ouverture.

## Acceptance Criteria

1. **Given** une opportunité de spread détectée (SpreadOpportunity sur channel)
   **When** l'exécution delta-neutral est déclenchée
   **Then** un ordre long est placé sur Exchange A
   **And** un ordre short est placé sur Exchange B en parallèle
   **And** les deux ordres sont envoyés dans une latence < 500ms (NFR2)
   **And** un log `[TRADE] Entry executed: spread=X%, long=ExchA, short=ExchB` est émis

## Tasks / Subtasks

- [x] **Task 1**: Créer le module `src/core/execution.rs` (AC: #1)
  - [x] Subtask 1.1: Créer le fichier `src/core/execution.rs`
  - [x] Subtask 1.2: Ajouter `pub mod execution;` dans `src/core/mod.rs`
  - [x] Subtask 1.3: Définir struct `DeltaNeutralExecutor` avec références aux deux adapters
  - [x] Subtask 1.4: Implémenter `DeltaNeutralExecutor::new(vest: VestAdapter, paradex: ParadexAdapter)`

- [x] **Task 2**: Implémenter `execute_delta_neutral()` (AC: #1)
  - [x] Subtask 2.1: Définir signature: `async fn execute_delta_neutral(&self, opportunity: SpreadOpportunity) -> ExchangeResult<DeltaNeutralResult>`
  - [x] Subtask 2.2: Créer `OrderRequest` pour chaque leg basé sur `SpreadDirection`
  - [x] Subtask 2.3: Exécuter les deux `place_order()` en parallèle avec `tokio::join!`
  - [x] Subtask 2.4: Mesurer latence totale avec `std::time::Instant`
  - [x] Subtask 2.5: Retourner `DeltaNeutralResult` avec statuts des deux legs

- [x] **Task 3**: Définir types de résultat (AC: #1)
  - [x] Subtask 3.1: Créer struct `DeltaNeutralResult` dans `execution.rs`
  - [x] Subtask 3.2: Champs: `long_order`, `short_order`, `execution_latency_ms`, `success`
  - [x] Subtask 3.3: Créer enum `LegStatus { Success(OrderResponse), Failed(ExchangeError) }`

- [x] **Task 4**: Implémenter logging structuré (AC: #1)
  - [x] Subtask 4.1: Log `[TRADE] Entry executed: spread=X%, long=ExchA, short=ExchB, latency=Yms`
  - [x] Subtask 4.2: Log d'erreur si une leg échoue: `[TRADE] Delta-neutral partial failure: long=success, short=failed`
  - [x] Subtask 4.3: Utiliser `tracing::info!` avec champs structurés

- [x] **Task 5**: Créer la task d'exécution dans le runtime (AC: #1)
  - [x] Subtask 5.1: Créer `src/core/runtime.rs` (nouveau fichier)
  - [x] Subtask 5.2: Définir `async fn execution_task(mut opportunity_rx: mpsc::Receiver<SpreadOpportunity>, executor: DeltaNeutralExecutor, shutdown: broadcast::Receiver<()>)`
  - [x] Subtask 5.3: Loop avec `tokio::select!` : shutdown en premier, puis opportunity_rx
  - [x] Subtask 5.4: Appeler `executor.execute_delta_neutral(opportunity)` pour chaque opportunité

- [x] **Task 6**: Tests unitaires (AC: #1)
  - [x] Subtask 6.1: `test_delta_neutral_executor_creation` - instanciation correcte
  - [x] Subtask 6.2: `test_execute_both_legs_parallel` - mock adapters, vérifier tokio::join! appelé
  - [x] Subtask 6.3: `test_execute_latency_measurement` - latence mesurée < seuil
  - [x] Subtask 6.4: `test_execute_one_leg_fails` - vérifier retour partial failure
  - [x] Subtask 6.5: `test_spread_direction_to_orders` - SpreadDirection::AOverB → Vest Buy, Paradex Sell

- [x] **Task 7**: Test de performance NFR2 (AC: #1)
  - [x] Subtask 7.1: `test_execution_latency_under_500ms` - covered by test_execute_latency_measurement
  - [x] Subtask 7.2: Mesurer latence réelle avec `Instant::elapsed()`
  - [x] Subtask 7.3: Assert latence < 500ms (marge: 100ms for mock)

- [x] **Task 8**: Validation finale (AC: #1)
  - [x] Subtask 8.1: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 8.2: `cargo test` tous les tests passent (184 tests)
  - [x] Subtask 8.3: Review des logs structurés avec spread, long, short fields

## Dev Notes

### 🔥 Contexte — Premier Story d'Exécution Delta-Neutral

> ⚠️ **CRITICAL**: Cette story implémente FR7 (exécution simultanée delta-neutral). C'est le cœur du bot HFT. Les Stories 2-1 et 2-2 ont déjà implémenté `place_order()` sur les deux adapters.

**Objectif: Créer le module d'exécution qui orchestre les deux ordres en parallèle.**

### Analyse du Code Existant

| Composant | Status | Fichier | Notes |
|-----------|--------|---------|-------|
| `VestAdapter::place_order()` | ✅ Done | `adapters/vest/adapter.rs` | EIP-712 signing, REST POST |
| `ParadexAdapter::place_order()` | ✅ Done | `adapters/paradex/adapter.rs` | SNIP-12 signing, REST POST |
| `SpreadOpportunity` | ✅ Existe | `core/channels.rs:18-25` | pair, dex_a, dex_b, spread_percent, direction |
| `SpreadDirection` | ✅ Existe | `core/spread.rs` | VestLong, ParadexLong |
| `ChannelBundle.opportunity_rx` | ✅ Existe | `core/channels.rs:31` | mpsc receiver pour opportunités |
| `execution.rs` | ❌ À créer | `core/execution.rs` | Module principal de cette story |
| `runtime.rs` | ❌ À créer | `core/runtime.rs` | Task orchestration |

### Architecture Guardrails

**Fichiers à créer :**
- `src/core/execution.rs` — DeltaNeutralExecutor, execute_delta_neutral(), types
- `src/core/runtime.rs` — execution_task loop

**Fichiers à modifier :**
- `src/core/mod.rs` — Ajouter `pub mod execution;` et `pub mod runtime;`

**Fichiers à NE PAS modifier :**
- `src/adapters/*/adapter.rs` — place_order() déjà implémenté
- `src/core/channels.rs` — SpreadOpportunity déjà défini
- `src/core/spread.rs` — SpreadDirection déjà défini

### 📋 Patterns Obligatoires

**Parallel Execution avec `tokio::join!` :**
```rust
// Pattern validé dans sprint-status.yaml comments (Story 2-2)
use tokio::join;

let (vest_result, paradex_result) = join!(
    self.vest_adapter.place_order(long_order),
    self.paradex_adapter.place_order(short_order)
);
```

**SpreadDirection Mapping :**
```rust
// Détermine quelle exchange reçoit le long vs short
match opportunity.direction {
    SpreadDirection::VestLong => {
        // Vest = Buy (Long), Paradex = Sell (Short)
        long_exchange = "vest";
        short_exchange = "paradex";
    }
    SpreadDirection::ParadexLong => {
        // Paradex = Buy (Long), Vest = Sell (Short)
        long_exchange = "paradex";
        short_exchange = "vest";
    }
}
```

**OrderRequest Construction :**
```rust
// Utiliser le builder existant (types.rs)
let order = OrderRequest::market(
    client_order_id,
    symbol.clone(),
    OrderSide::Buy,  // ou Sell pour short
    quantity,
);
```

**DeltaNeutralResult Structure :**
```rust
#[derive(Debug, Clone)]
pub struct DeltaNeutralResult {
    pub long_order: LegStatus,
    pub short_order: LegStatus,
    pub execution_latency_ms: u64,
    pub success: bool,  // true si les deux legs ont réussi
}

#[derive(Debug, Clone)]
pub enum LegStatus {
    Success(OrderResponse),
    Failed(String),  // Error message
}
```

**Execution Task Loop :**
```rust
// Pattern établi avec shutdown prioritaire
async fn execution_task(
    mut opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    executor: DeltaNeutralExecutor,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Execution task shutting down");
                break;
            }
            Some(opportunity) = opportunity_rx.recv() => {
                match executor.execute_delta_neutral(opportunity).await {
                    Ok(result) => {
                        if result.success {
                            info!(
                                spread = %result.spread_percent,
                                long = %result.long_exchange,
                                short = %result.short_exchange,
                                latency_ms = result.execution_latency_ms,
                                "Entry executed"
                            );
                        }
                    }
                    Err(e) => error!(error = ?e, "Delta-neutral execution failed"),
                }
            }
        }
    }
}
```

### ⏱️ NFR2 — Latency Constraint

**NFR2:** Detection-to-order latency < 500ms

- L'exécution parallèle réduit la latence à max(vest_latency, paradex_latency)
- Sprint-status note ~250ms pour Paradex (99.9% HTTP, crypto négligeable)
- Vest devrait être similaire
- Avec `tokio::join!`, la latence totale devrait être ~250-300ms

**Mesure de latence :**
```rust
let start = std::time::Instant::now();
let (vest_result, paradex_result) = join!(...);
let latency_ms = start.elapsed().as_millis() as u64;
```

### Previous Story Intelligence

**Story 2-1 (Order Long) — DONE :**
- `place_order()` Vest avec EIP-712 signing implémenté
- `place_order()` Paradex avec SNIP-12 signing implémenté
- Log structuré: `info!(pair=%pair, side="long", size=%qty, "Order placed")`

**Story 2-2 (Order Short) — DONE (inline notes) :**
- Short orders testés sur Paradex avec `reduce_only=true`
- Full lifecycle: open LONG → get_position → close with SELL reduce_only
- Both Vest and Paradex use REST API (parallel execution via `tokio::join!`)

**Story 2-0 (Adapter Refactoring) — DONE :**
- Structure modulaire: vest/{mod, config, types, signing, adapter}
- 174 tests passent, clippy clean
- Signature latency: ~0.16ms en release

### Git Commit Pattern

Préfixe: `feat(story-2.3):` pour les nouvelles fonctionnalités
Exemple: `feat(story-2.3): Implement DeltaNeutralExecutor with parallel order execution`

### Project Structure Post-Implementation

```
src/core/
├── mod.rs           # + pub mod execution; pub mod runtime;
├── channels.rs      # SpreadOpportunity (existant)
├── spread.rs        # SpreadDirection (existant)
├── vwap.rs          # VWAP engine (existant)
├── execution.rs     # DeltaNeutralExecutor (NOUVEAU) ✅
└── runtime.rs       # execution_task loop (NOUVEAU) ✅
```

### Technical Requirements

**Imports nécessaires dans execution.rs :**
```rust
use std::time::Instant;
use tokio::join;
use tracing::{info, error, warn};

use crate::adapters::{
    ExchangeAdapter,
    types::{OrderRequest, OrderResponse, OrderSide},
    vest::VestAdapter,
    paradex::ParadexAdapter,
};
use crate::core::channels::{SpreadOpportunity, SpreadDirection};
use crate::adapters::errors::ExchangeResult;
```

**Imports nécessaires dans runtime.rs :**
```rust
use tokio::sync::{mpsc, broadcast};
use tracing::{info, error};

use crate::core::channels::SpreadOpportunity;
use crate::core::execution::DeltaNeutralExecutor;
```

### ⚠️ Points d'Attention Critiques

1. **Ownership des Adapters** : `DeltaNeutralExecutor` doit posséder ou avoir des références aux adapters. Utiliser des `Arc<Mutex<Adapter>>` si nécessaire pour le sharing entre tasks.

2. **Error Handling** : Si une leg échoue, retourner `DeltaNeutralResult` avec status partiel. Story 2-5 (auto-close) gérera le rollback.

3. **Symbol Mapping** : Les symbols diffèrent entre exchanges:
   - Vest: `BTC-PERP`
   - Paradex: `BTC-USD-PERP`
   - Le mapping doit être géré (config ou hardcodé pour MVP)

4. **Quantity Calculation** : Pour MVP, utiliser une quantity fixe ou configurée. L'optimisation VWAP viendra plus tard.

5. **Idempotency** : Chaque ordre a un `client_order_id` unique. Utiliser UUID ou timestamp-based.

### References

- [Source: architecture.md#Execution] — FR7 Exécution simultanée delta-neutral, NFR2 <500ms
- [Source: architecture.md#Runtime] — Multi-Task Pipeline, tokio::select! patterns
- [Source: epics.md#Story 2.3] — Acceptance criteria originaux
- [Source: channels.rs#SpreadOpportunity] — Struct détaillée (L18-25)
- [Source: channels.rs#opportunity_rx] — Channel receiver pour trigger (L31)
- [Source: sprint-status.yaml#2-2] — Notes sur parallel execution avec tokio::join!
- [Source: 2-1-placement-ordre-long.md] — Pattern place_order et signing

## Definition of Done Checklist

- [ ] Code compiles sans warnings (`cargo build`)
- [ ] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [ ] Tests passent (`cargo test`)
- [ ] `src/core/execution.rs` créé avec DeltaNeutralExecutor
- [ ] `src/core/runtime.rs` créé avec execution_task
- [ ] `tokio::join!` utilisé pour exécution parallèle
- [ ] Latence mesurée et loggée
- [ ] Logs structurés: spread, long, short, latency_ms
- [ ] Tests unitaires pour execution et runtime
- [ ] Test de performance NFR2 (<500ms)

## Dev Agent Record

### Agent Model Used

{{agent_model_name_version}}

### Debug Log References

### Completion Notes List

### File List
