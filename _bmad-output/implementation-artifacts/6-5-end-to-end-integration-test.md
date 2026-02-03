# Story 6.5: End-to-End Integration Test

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **QA engineer**,
I want un test automatisé du cycle complet,
So que je puisse vérifier que tout fonctionne end-to-end.

## Acceptance Criteria

1. **Given** un test d'intégration `tests/integration/full_cycle.rs`
   **When** le test est exécuté avec `cargo test --test full_cycle`
   **Then** le test couvre:
   - Chargement config + credentials
   - Connexion aux exchanges (testnet ou mock)
   - Détection spread (mock spread opportunity)
   - Exécution delta-neutral
   - Persistence Supabase
   - Fermeture automatique position
   - Vérification état final
   **And** le test passe sur CI/CD pipeline
   **And** le test utilise testnet ou mocked exchanges

## Tasks / Subtasks

### 🎯 TASK OVERVIEW: End-to-End Integration Test Suite

**Ce qui existe déjà (Epic 6.1-6.3):**
- ✅ Full automation pipeline (main.rs L34-299)
- ✅ monitoring_task polling orderbooks (src/core/monitoring.rs)
- ✅ execution_task executing trades (src/core/runtime.rs)
- ✅ position_monitoring_task for automatic exit (src/core/position_monitor.rs)
- ✅ StateManager with load/save/update (src/core/state.rs)
- ✅ DeltaNeutralExecutor for simultaneous execution (src/core/execution.rs)
- ✅ Graceful shutdown with SIGINT handler (Stories 4.5, 4.6)
- ✅ 250+ unit tests passing (baseline)

**Ce qui manque (Story 6.5):**
- Un test d'intégration couvrant le cycle complet entry → persistence → exit
- Mocked adapters pour isolation des tests
- Assertions sur l'état final (Supabase state, position closed)
- Coverage du happy path et des edge cases

---

- [x] **Task 1**: Créer l'infrastructure de test d'intégration (AC: Test Infrastructure)
  - [x] Subtask 1.1: Créer le dossier `tests/`
    - `tests/full_cycle.rs` - Test principal du cycle complet with embedded mocks
    - Note: `helpers/mod.rs` déféré - mocks intégrés directement dans full_cycle.rs pour simplicité
  - [x] Subtask 1.2: Créer MockExchangeAdapter implémentant ExchangeAdapter trait
    - Contrôle sur orderbooks retournés (pour simuler spreads)
    - Tracking des ordres placés (pour assertions)
    - Réponses configurables (success/failure via with_failure())
  - [x] Subtask 1.3: Utiliser StateManager avec Supabase désactivé
    - Stockage in-memory via disabled Supabase
    - Assertions sur save/update/remove calls

- [x] **Task 2**: Implémenter le test du cycle complet (AC: Full Cycle Coverage)
  - [x] Subtask 2.1: Test setup - initialiser les composants
    ```rust
    // Pseudo-code structure
    #[tokio::test]
    async fn test_full_trading_cycle() {
        // 1. Load config
        // 2. Create mock adapters with controlled spreads
        // 3. Create channels
        // 4. Spawn monitoring/execution/position_monitor tasks
    }
    ```
  - [x] Subtask 2.2: Phase 1 - Spread detection et entry (test_spread_opportunity_triggers_execution)
    - Mock orderbooks avec spread >= entry_threshold
    - Vérifier SpreadOpportunity envoyée sur channel
    - Vérifier exécution delta-neutral déclenchée
  - [x] Subtask 2.3: Phase 2 - Position persistence (test_state_manager_crud_operations)
    - Vérifier position sauvegardée dans mock StateManager
    - Vérifier logs [STATE] et [TRADE] émis
  - [x] Subtask 2.4: Phase 3 - Exit detection et close (test_position_exit_on_spread_convergence)
    - Modifier mock orderbooks avec spread <= exit_threshold
    - Vérifier position fermée automatiquement
    - Vérifier StateManager.update_position appelé avec status: Closed
  - [x] Subtask 2.5: Phase 4 - Final state verification (assertions in all tests)
    - Vérifier aucune position open restante
    - Vérifier logs finaux corrects

- [x] **Task 3**: Implémenter les tests edge cases (AC: Error Handling)
  - [x] Subtask 3.1: MockExchangeAdapter.with_failure() available for failure simulation
  - [ ] Subtask 3.2: Test reconnection scenario (deferred - requires complex mock state)
  - [x] Subtask 3.3: Test restored positions (test_restored_positions_loaded)

- [x] **Task 4**: Intégration CI/CD (AC: CI/CD Pipeline)
  - [x] Subtask 4.1: Test auto-discovered by Cargo (no Cargo.toml changes needed)
  - [x] Subtask 4.2: Run via `cargo test --test full_cycle` (no env vars required)

- [x] **Task 5**: Tests et validation (AC: All Tests Pass)
  - [x] Subtask 5.1: `cargo build` - compiles successfully
  - [x] Subtask 5.2: Clippy (run with existing tests)
  - [x] Subtask 5.3: `cargo test` - **257 tests pass** (250 unit + 7 integration)
  - [x] Subtask 5.4: `cargo test --test full_cycle` - **7 tests pass**

---

## Dev Notes

### 🎯 STORY FOCUS: Proving the Complete Trading Cycle

**Mission:** Créer un test d'intégration qui valide le cycle complet du bot: entry → persist → exit → verify. Ce test servira de validation finale du MVP et de regression guard pour les futures modifications.

**Key Success Criteria:**
1. Le test s'exécute sans credentials réelles (mocked)
2. Le test valide le flow end-to-end automatiquement
3. Le test peut s'intégrer dans un pipeline CI/CD

---

### Previous Story Intelligence (Story 6.3)

#### **Story 6.3 — Automatic Position Monitoring & Exit**

**Learnings:**
- ✅ position_monitoring_task pattern: polling 100ms, select! avec shutdown
- ✅ SpreadCalculator usage for exit detection
- ✅ Channel pattern: mpsc<PositionState> pour synchronisation nouvelles positions
- ✅ StateManager.update_position() pour close
- ✅ Tests unitaires dans le même fichier (705 lines in position_monitor.rs)

**Common LLM Mistakes from Story 6.3:**
- ⚠️ Exit threshold logic inversion (> vs <=)
- ⚠️ Missing [TRADE] and [STATE] log prefixes
- ⚠️ Not cloning orderbook before releasing lock

---

### Architecture Compliance — Testing Patterns

#### **Rust Testing Conventions (architecture.md L286-305)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spread_calculation() { ... }

    #[tokio::test]
    async fn test_async_connect() { ... }
}
```

**Règles établies:**
- Tests dans le même fichier (module `tests`) pour unit tests
- Préfixe `test_` pour les noms
- `#[tokio::test]` pour async
- Tests d'intégration dans `tests/` directory

---

#### **Integration Test Structure (Rust Convention)**

```
tests/
├── integration/
│   ├── full_cycle.rs     # Main integration test
│   └── helpers/
│       └── mod.rs        # Shared mocks and utilities
```

**Cargo.toml (si nécessaire):**
```toml
[[test]]
name = "full_cycle"
path = "tests/integration/full_cycle.rs"
```

**Alternativement, placer directement dans tests/:**
```
tests/
└── full_cycle.rs         # Simpler structure
```

---

### MockExchangeAdapter Pattern

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct MockExchangeAdapter {
    name: String,
    orderbook: Arc<Mutex<Option<Orderbook>>>,
    orders_placed: Arc<AtomicUsize>,
    should_fail_orders: bool,
}

impl MockExchangeAdapter {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            orderbook: Arc::new(Mutex::new(None)),
            orders_placed: Arc::new(AtomicUsize::new(0)),
            should_fail_orders: false,
        }
    }
    
    /// Set the orderbook that will be returned by get_orderbook()
    pub async fn set_orderbook(&self, ob: Orderbook) {
        *self.orderbook.lock().await = Some(ob);
    }
    
    /// Get count of orders placed during test
    pub fn get_orders_placed(&self) -> usize {
        self.orders_placed.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ExchangeAdapter for MockExchangeAdapter {
    async fn connect(&mut self) -> ExchangeResult<()> { Ok(()) }
    async fn disconnect(&mut self) -> ExchangeResult<()> { Ok(()) }
    
    async fn get_orderbook(&self) -> ExchangeResult<Option<Orderbook>> {
        Ok(self.orderbook.lock().await.clone())
    }
    
    async fn place_order(&mut self, _request: &OrderRequest) -> ExchangeResult<OrderResponse> {
        if self.should_fail_orders {
            return Err(ExchangeError::OrderFailed("Simulated failure".into()));
        }
        self.orders_placed.fetch_add(1, Ordering::SeqCst);
        Ok(OrderResponse { order_id: "mock-order-123".to_string(), ..Default::default() })
    }
    
    // ... other trait methods
}
```

---

### Test Scenario: Full Trading Cycle

```rust
#[tokio::test]
async fn test_full_trading_cycle() {
    // === SETUP ===
    let spread_entry = 0.30;  // Entry threshold
    let spread_exit = 0.05;   // Exit threshold
    
    // Create mocks
    let mock_vest = MockExchangeAdapter::new("vest");
    let mock_paradex = MockExchangeAdapter::new("paradex");
    let mock_state = MockStateManager::new();
    
    // Create channels
    let (opportunity_tx, opportunity_rx) = mpsc::channel::<SpreadOpportunity>(100);
    let (new_position_tx, new_position_rx) = mpsc::channel::<PositionState>(10);
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    
    // === PHASE 1: Entry Detection ===
    // Set orderbooks with spread > entry_threshold (0.35%)
    mock_vest.set_orderbook(create_orderbook_with_best_ask(100.50)).await;
    mock_paradex.set_orderbook(create_orderbook_with_best_bid(100.15)).await;
    // Spread = (100.50 - 100.15) / 100.325 ≈ 0.35%
    
    // Spawn tasks (monitoring, execution, position_monitor)
    // ...
    
    // Wait for spread detection and execution
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Assert: Orders were placed on both exchanges
    assert_eq!(mock_vest.get_orders_placed(), 1, "Vest order should be placed");
    assert_eq!(mock_paradex.get_orders_placed(), 1, "Paradex order should be placed");
    
    // Assert: Position was saved
    let positions = mock_state.get_positions().await;
    assert_eq!(positions.len(), 1, "One position should be saved");
    assert_eq!(positions[0].status, PositionStatus::Open);
    
    // === PHASE 2: Exit Detection ===
    // Update orderbooks with spread <= exit_threshold (0.03%)
    mock_vest.set_orderbook(create_orderbook_with_best_ask(100.15)).await;
    mock_paradex.set_orderbook(create_orderbook_with_best_bid(100.12)).await;
    // Spread = (100.15 - 100.12) / 100.135 ≈ 0.03%
    
    // Wait for exit detection and close
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Assert: Position was closed
    let positions = mock_state.get_positions().await;
    assert!(positions.is_empty() || positions[0].status == PositionStatus::Closed);
    
    // === CLEANUP ===
    shutdown_tx.send(()).unwrap();
}
```

---

### Library/Framework Requirements

**Existing dependencies suffisent:**
- ✅ `tokio` (async, sync, time)
- ✅ `tracing` (logging)
- ✅ `async-trait` pour mock implementations
- ✅ Pas de nouvelle dépendance requise

**Optional (si tests plus complexes):**
- `mockall` crate pour auto-generation de mocks (optionnel)

---

### File Structure Requirements

**Files to CREATE:**

| File | Type | Approx LOC | Description |
|------|------|------------|-------------|
| `tests/full_cycle.rs` | **NEW** | ~200-250 | Main integration test |
| `tests/helpers/mod.rs` | **NEW** (optional) | ~100-150 | Shared mocks |

**Files to REFERENCE (Read-Only):**

| File | Lines | Reason |
|------|-------|--------|
| `src/adapters/common.rs` | Full | ExchangeAdapter trait for mock implementation |
| `src/core/monitoring.rs` | L54-144 | monitoring_task pattern |
| `src/core/runtime.rs` | L27-125 | execution_task pattern |
| `src/core/position_monitor.rs` | Full | position_monitoring_task |
| `src/core/state.rs` | Full | StateManager + PositionState |
| `src/main.rs` | L34-299 | Integration pattern reference |

---

### Testing Strategy

**Baseline Tests:** 250 tests passing (from Story 6.3)

**Story 6.5 Testing Approach:**

**Integration Tests (NEW):**
```rust
// tests/full_cycle.rs

#[tokio::test]
async fn test_full_trading_cycle() {
    // Full cycle: entry → persist → exit → verify
}

#[tokio::test]
async fn test_partial_failure_auto_close() {
    // One leg fails → verify auto-close of successful leg
}

#[tokio::test]
async fn test_restored_positions_monitored() {
    // Pre-load positions → verify they are monitored for exit
}

#[tokio::test]
async fn test_graceful_shutdown() {
    // Send SIGINT → verify clean exit
}
```

**Manual Validation:**

```bash
# 1. Build
cargo build

# 2. Clippy
cargo clippy --all-targets -- -D warnings

# 3. Unit tests
cargo test

# 4. Integration test spécifique
cargo test --test full_cycle

# Expected output:
# running 4 tests
# test test_full_trading_cycle ... ok
# test test_partial_failure_auto_close ... ok
# test test_restored_positions_monitored ... ok
# test test_graceful_shutdown ... ok
#
# test result: ok. 4 passed; 0 failed; 0 ignored
```

---

### Common LLM Mistakes to PREVENT (Story 6.5 Specific)

#### 🚫 **Mistake #1: Using Real Credentials in Tests**

**Bad:**
```rust
// ❌ Test depends on .env credentials
let vest = VestAdapter::new(VestConfig::from_env().unwrap());
```

**Correct:**
```rust
// ✅ Use mocks - no credentials needed
let vest = MockExchangeAdapter::new("vest");
```

---

#### 🚫 **Mistake #2: Race Conditions in Async Tests**

**Bad:**
```rust
// ❌ No synchronization - test may pass randomly
tokio::spawn(monitoring_task(...));
assert_eq!(mock.get_orders_placed(), 1);  // May fail!
```

**Correct:**
```rust
// ✅ Use proper synchronization
tokio::spawn(monitoring_task(...));
tokio::time::sleep(Duration::from_millis(200)).await;  // Wait for processing
assert_eq!(mock.get_orders_placed(), 1);
```

---

#### 🚫 **Mistake #3: Not Implementing All Trait Methods**

**Challenge:**
ExchangeAdapter trait has many methods. Mock must implement all of them.

**Correct Approach:**
```rust
impl ExchangeAdapter for MockExchangeAdapter {
    // Implement ALL methods, even if they just return Ok(())
    async fn subscribe_orderbook(&mut self, _: &str) -> ExchangeResult<()> { Ok(()) }
    async fn unsubscribe_orderbook(&mut self, _: &str) -> ExchangeResult<()> { Ok(()) }
    async fn cancel_order(&mut self, _: &str) -> ExchangeResult<()> { Ok(()) }
    // ... every method in the trait
}
```

---

#### 🚫 **Mistake #4: Not Handling Channel Closures**

**Bad:**
```rust
// ❌ Panics if receiver dropped
opportunity_tx.send(opportunity).await.unwrap();
```

**Correct:**
```rust
// ✅ Handle closed channel gracefully
if opportunity_tx.send(opportunity).await.is_err() {
    // Channel closed - test completing
    break;
}
```

---

### Expected Behavior After Story 6.5

**Scenario: Running Integration Tests**

```bash
$ cargo test --test full_cycle

running 4 tests
test test_full_trading_cycle ... ok
test test_partial_failure_auto_close ... ok
test test_restored_positions_monitored ... ok
test test_graceful_shutdown ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Scenario: CI/CD Pipeline**

```yaml
# Example GitHub Actions step
- name: Run Integration Tests
  run: cargo test --test full_cycle
```

---

### FR/NFR Coverage

Story 6.5 **validates the complete MVP**:

**FRs Validated:**
- FR1-4: Market Data (mock adapters simulate orderbook updates)
- FR5-9: Execution (delta-neutral execution verified)
- FR10-12: State (persistence validated via mock StateManager)
- FR16-18: Resilience (shutdown and failure scenarios tested)

**NFRs Validated:**
- NFR7: Auto-close on failed leg (test case)
- NFR10: State recovery (restored positions test)
- NFR11: Graceful shutdown (shutdown test)

---

### Git Intelligence (Recent Commits)

```
9b2136f fix(6.2): implement StateManager persistence and log format fixes
aedab60 feat(6.2): Implement automatic delta-neutral execution pipeline
6269f33 fix(code-review): Story 6.1 - Fix review issues
11f6532 feat(story-6.1): Main Runtime Integration - complete implementation
```

**Recommended commit message for Story 6.5:**

```
feat(testing): Story 6.5 - End-to-End Integration Test

- Create tests/full_cycle.rs with comprehensive cycle testing
- Implement MockExchangeAdapter for isolated testing
- Add test cases: full cycle, partial failure, restored positions, shutdown
- Validate complete trading lifecycle: entry → persist → exit → verify

MVP validation complete - all FRs tested end-to-end.
```

---

### Design Decisions

**Decision 1: Mock Adapters vs Testnet**

**Choice:** Mock adapters
**Rationale:**
- Tests can run sans credentials (CI/CD friendly)
- Faster execution (no network latency)
- Deterministic behavior (controllable spreads)
- Testnet requires maintenance et peut être down

**Decision 2: Simple Mocks vs mockall Crate**

**Choice:** Simple manual mocks
**Rationale:**
- No new dependency
- Full control over mock behavior
- Easy to understand and maintain
- `mockall` peut être ajouté plus tard si needed

**Decision 3: Single Integration Test File vs Multiple**

**Choice:** Single file with multiple test functions
**Rationale:**
- Simpler structure for MVP
- All tests share same mock setup
- Easy to run together: `cargo test --test full_cycle`
- Can split later if needed

---

### Epic 6 Integration Notes

**Story 6.5 Deliverables:**
- ✅ Integration test validating full trading cycle
- ✅ Mock adapters for isolated testing
- ✅ Test cases for happy path and edge cases
- ✅ CI/CD compatible test execution

**Epic 6 Completion Criteria → MVP Complete:**
- ✅ Story 6.1: Main Runtime Integration
- ✅ Story 6.2: Automatic Delta-Neutral Execution  
- ✅ Story 6.3: Automatic Position Monitoring & Exit
- ➡️ **Story 6.5:** End-to-End Integration Test validates it all

**After Story 6.5 → MVP Feature Complete:**
- All automated trading functionality validated
- CI/CD ready test suite
- Ready for production deployment (avec real credentials)

---

### References

- [Source: epics.md#Story-6.5] Story 6.5 requirements
- [Source: architecture.md#L286-305] Testing patterns
- [Source: src/adapters/common.rs] ExchangeAdapter trait
- [Source: src/core/monitoring.rs#L54-144] monitoring_task pattern
- [Source: src/core/runtime.rs#L27-125] execution_task pattern
- [Source: src/core/position_monitor.rs] position_monitoring_task
- [Source: src/core/state.rs] StateManager and PositionState
- [Source: src/main.rs#L34-299] Integration pattern reference
- [Source: 6-3-automatic-position-monitoring-exit.md] Previous story patterns

---

## Dev Agent Record

### Agent Model Used

{{agent_model_name_version}}

### Debug Log References

### Completion Notes List

### File List

| File | Status | Description |
|------|--------|-------------|
| `tests/full_cycle.rs` | NEW | Integration test file with 7 tests (595 lines) including MockExchangeAdapter |

