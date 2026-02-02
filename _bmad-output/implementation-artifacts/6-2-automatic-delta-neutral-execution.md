# Story 6.2: Automatic Delta-Neutral Execution

Status: review

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **opérateur**,
I want que les positions s'ouvrent automatiquement quand spread ≥ threshold,
So that je ne rate pas d'opportunités de trading.

## Acceptance Criteria

1. **Given** le monitoring de spreads est actif  
   **When** spread ≥ `spread_entry` threshold configuré  
   **Then** exécution delta-neutral est **automatiquement déclenchée**  
   **And** aucune intervention manuelle requise  
   **And** la logique de Story 2.3 (simultaneous long/short) est utilisée  
   **And** en cas de succès, position persistée dans Supabase (Story 3.2)  
   **And** en cas d'échec d'un leg, auto-close logic déclenchée (Story 2.5)  
   **And** un log `[TRADE] Auto-executed: spread=X%` est émis

2. **Given** les connexions WebSocket actives  
   **When** les orderbooks sont mis à jour  
   **Then** le spread est calculé en continu  
   **And** le calcul s'exécute en <2ms (NFR1)

3. **Given** une opportunité déclenchée  
   **When** l'exécution est réussie sur les deux legs  
   **Then** la position est sauvegardée dans Supabase (StateManager)  
   **And** le log `[STATE] Position saved` est émis

## Tasks / Subtasks

### 🎯 TASK OVERVIEW: Automatic Delta-Neutral Execution

**Ce qui existe déjà (Story 6.1 + Epics 1-4):**
- ✅ main.rs scaffold avec connexions et shutdown (Story 6.1)
- ✅ DeltaNeutralExecutor complet (Stories 2.3-2.5)
- ✅ SpreadCalculator avec `calculate()` (Stories 1.4-1.5)
- ✅ execution_task pattern (src/core/runtime.rs)
- ✅ SpreadOpportunity struct (src/core/channels.rs)
- ✅ StateManager avec save/load (Stories 3.1-3.4)
- ✅ Shutdown broadcast channel (Story 4.5)
- ✅ Orderbook subscription (Story 1.3)

**Ce qui manque (Story 6.2):**
- Monitoring task qui poll orderbooks et calcule spreads
- Connection du monitoring_task au execution_task via channel
- Spawn des deux tasks dans main.rs
- Persistence des positions avec StateManager après trade

---

- [x] **Task 1**: Créer le monitoring_task (AC: Spread Monitoring)
  - [x] Subtask 1.1: Créer `src/core/monitoring.rs` avec `monitoring_task()`
    - Signature: `async fn monitoring_task<V, P>(vest: Arc<Mutex<V>>, paradex: Arc<Mutex<P>>, opportunity_tx: mpsc::Sender<SpreadOpportunity>, config: MonitoringConfig, shutdown_rx: broadcast::Receiver<()>) where V: ExchangeAdapter, P: ExchangeAdapter`
    - Loop infini avec shutdown check via `tokio::select!`
    - Pattern: identique à execution_task (runtime.rs L33-73)
  - [x] Subtask 1.2: Implémenter le polling d'orderbooks
    - Poll les orderbooks des deux adapters via `adapter.lock().await.get_orderbook(symbol)`
    - Intervalle: 100ms (tokio::time::interval)
    - Continuer si orderbook None (pas encore de données)
  - [x] Subtask 1.3: Calculer le spread avec SpreadCalculator
    - Créer `SpreadCalculator::new("vest", "paradex")`
    - Appeler `calculator.calculate(&vest_orderbook, &paradex_orderbook)`
    - Log si spread calculé: `tracing::debug!("Spread: {:.4}%", spread_result.spread_pct)`
  - [x] Subtask 1.4: Détecter le dépassement de seuil
    - Comparer `spread_result.spread_pct >= config.spread_entry`
    - Log: `[TRADE] Spread opportunity detected: spread=X%, threshold=Y%`
  - [x] Subtask 1.5: Envoyer SpreadOpportunity sur le channel
    - Créer `SpreadOpportunity` avec pair, dex_a, dex_b, spread_percent, direction, timestamp
    - `opportunity_tx.try_send(opportunity)` (non-blocking send)

- [x] **Task 2**: Spawn monitoring_task dans main.rs (AC: Integration)
  - [x] Subtask 2.1: Importer monitoring_task dans main.rs
    - `use hft_bot::core::monitoring::{monitoring_task, MonitoringConfig};`
    - Ajouter export dans `src/core/mod.rs`
  - [x] Subtask 2.2: Connecter opportunity_tx et opportunity_rx
    - Renommé `_opportunity_tx` → `opportunity_tx` (L107)
    - `_opportunity_rx` reste unused pour Story 6.2 Task 3
  - [x] Subtask 2.3: Spawn monitoring_task avant le shutdown wait
    - Créé `MonitoringConfig { pair, spread_entry }` depuis BotConfig
    - `tokio::spawn(async move { monitoring_task(...).await })`
    - Inséré après subscribe_orderbook, avant SIGINT handler

- [x] **Task 3**: Spawn execution_task dans main.rs (AC: Automatic Execution)
  - [x] Subtask 3.1: Importer execution_task et DeltaNeutralExecutor
    - `use hft_bot::core::runtime::execution_task;`
    - `use hft_bot::core::execution::DeltaNeutralExecutor;`
  - [x] Subtask 3.2: Créer DeltaNeutralExecutor avec adapters séparés
    - Créé nouvelles instances via `VestConfig::from_env()` et `ParadexConfig::from_env()`
    - `position_size`: 0.001 pour MVP (TODO: calculer depuis config.capital)
    - Symbols: vest_symbol, paradex_symbol
  - [x] Subtask 3.3: Spawn execution_task
    - Clone `shutdown_tx.subscribe()` pour execution
    - `tokio::spawn(async move { execution_task(opportunity_rx, executor, shutdown_rx).await })`

- [ ] **Task 4**: Intégrer persistence après trade (AC: Supabase Save)
  - [ ] Subtask 4.1: Modifier execution_task pour accepter StateManager
    - Ajouter paramètre `state_manager: Arc<StateManager>` à execution_task
    - OU créer wrapper dans main.rs qui appelle save après execute
  - [ ] Subtask 4.2: Sauvegarder position après trade réussi
    - Si `result.success` → `state_manager.save_position(position).await`
    - Log `[STATE] Position saved: pair=X, entry_spread=Y%`
  - [ ] Subtask 4.3: Gérer erreur de persistence (warn + continue)
    - Ne pas bloquer trading si Supabase échoue
    - Log `[STATE] Failed to save position: {error}. Trading continues.`

- [/] **Task 5**: Tests et validation (AC: All Tests Pass)
  - [x] Subtask 5.1: `cargo build` - code compile sans warnings
  - [x] Subtask 5.2: `cargo clippy --all-targets -- -D warnings` - 0 warnings
  - [x] Subtask 5.3: `cargo test` - baseline tests passent (244 passed, 0 failed)
  - [x] Subtask 5.4: Ajouter tests unitaires pour monitoring_task
    - Test: monitoring_task shutdown proprement sur signal ✅
    - Test: SpreadOpportunity envoyé quand spread > threshold ✅
    - Test: No opportunity below threshold ✅
  - [ ] Subtask 5.5: Manual test - spread opportunity triggers trade
    - Requiert Tasks 3-4 completion (execution_task spawn)

---

## Dev Notes

### 🎯 STORY FOCUS: Automatic Execution Pipeline (Epic 6.2)

**Mission:** Connecter le monitoring de spreads à l'exécution automatique pour créer le bot fully autonomous.

**Key Integration Points:**
1. **monitoring_task** → poll orderbooks → calculate spread → detect opportunity
2. **SpreadOpportunity** channel → transporte opportunités vers executor
3. **execution_task** → consume opportunity → execute delta-neutral
4. **StateManager** → persist successful trades

---

### Previous Story Intelligence (Story 6.1)

#### **Story 6.1 — Main Runtime Integration**

**Learnings:**
- ✅ **TODOs documentés**: main.rs L107-163 marque exactement où intégrer
- ✅ **Channels déjà créés**: `_opportunity_tx`, `_opportunity_rx` (L108)
- ✅ **Shutdown pattern**: `shutdown_tx.subscribe()` pour chaque task
- ✅ **Symbol mapping**: vest_symbol, paradex_symbol définis L139-142
- ✅ **Adapters wrappés**: `Arc<Mutex<VestAdapter>>`, `Arc<Mutex<ParadexAdapter>>`

**Pattern Continuity for Story 6.2:**
- L107-108: Remplacer `_` prefix pour activer channels
- L151-163: Retirer TODO comments, implémenter actual spawns
- Ajouter monitoring_task spawn entre L165 et L167

**Common LLM Mistakes from Story 6.1:**
- ⚠️ Forgetting `.clone()` before passing Arc to spawned tasks
- ⚠️ Not using `.subscribe()` for each shutdown receiver
- ⚠️ Missing `.await` on Mutex lock

---

### Architecture Compliance — Monitoring Task Pattern

#### **execution_task Pattern (src/core/runtime.rs L23-76)**

```rust
pub async fn execution_task<V, P>(
    mut opportunity_rx: mpsc::Receiver<SpreadOpportunity>,
    executor: DeltaNeutralExecutor<V, P>,
    mut shutdown_rx: broadcast::Receiver<()>,
) where
    V: ExchangeAdapter + Send + Sync,
    P: ExchangeAdapter + Send + Sync,
{
    info!("Execution task started");

    loop {
        tokio::select! {
            // Shutdown takes priority
            _ = shutdown_rx.recv() => {
                info!("Execution task shutting down");
                break;
            }
            // Process incoming opportunities
            Some(opportunity) = opportunity_rx.recv() => {
                // ... execute trade
            }
        }
    }

    info!("Execution task stopped");
}
```

**→ monitoring_task doit suivre exactement ce même pattern.**

---

#### **SpreadCalculator Usage (src/core/spread.rs)**

```rust
use hft_bot::core::spread::SpreadCalculator;
use hft_bot::adapters::types::Orderbook;

let calculator = SpreadCalculator::new("vest", "paradex");

// Get orderbooks from adapters
if let (Some(vest_ob), Some(paradex_ob)) = (
    vest.lock().await.get_orderbook(&vest_symbol),
    paradex.lock().await.get_orderbook(&paradex_symbol),
) {
    if let Some(spread_result) = calculator.calculate(vest_ob, paradex_ob) {
        // spread_result.spread_pct contains the percentage
        // spread_result.direction contains AOverB or BOverA
    }
}
```

**⚠️ CRITICAL:** `get_orderbook()` returns reference → must clone or use immediately

---

#### **SpreadOpportunity Creation (src/core/channels.rs L18-25)**

```rust
pub struct SpreadOpportunity {
    pub pair: String,
    pub dex_a: String,
    pub dex_b: String,
    pub spread_percent: f64,
    pub direction: SpreadDirection,
    pub detected_at_ms: u64,
}

// Creating opportunity
let now_ms = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0);

let opportunity = SpreadOpportunity {
    pair: "BTC-PERP".to_string(),
    dex_a: "vest".to_string(),
    dex_b: "paradex".to_string(),
    spread_percent: spread_result.spread_pct,
    direction: spread_result.direction,
    detected_at_ms: now_ms,
};
```

---

### Library/Framework Requirements — No New Dependencies

**Existing dependencies suffisent:**
- ✅ `tokio` (async, sync, time::interval)
- ✅ `tracing` (logging)
- ✅ `mpsc` channel déjà créé dans main.rs

**Tokio time::interval usage:**
```rust
use tokio::time::{interval, Duration};

let mut poll_interval = interval(Duration::from_millis(100));
loop {
    tokio::select! {
        _ = shutdown_rx.recv() => { break; }
        _ = poll_interval.tick() => {
            // Poll orderbooks and calculate spread
        }
    }
}
```

---

### File Structure Requirements

**Files to CREATE:**

| File | Type | Approx LOC | Description |
|------|------|------------|-------------|
| `src/core/monitoring.rs` | **NEW** | ~80-100 | monitoring_task function |

**Files to MODIFY:**

| File | Type | Approx LOC Change | Description |
|------|------|-------------------|-------------|
| `src/core/mod.rs` | **MODIFY** | +1 | Export monitoring module |
| `src/main.rs` | **MODIFY** | +30-40 | Spawn monitoring_task + execution_task |

**Files to REFERENCE (Read-Only):**

| File | Lines | Reason |
|------|-------|--------|
| `src/core/runtime.rs` | L23-76 | execution_task pattern (clone this for monitoring) |
| `src/core/spread.rs` | L68-131 | SpreadCalculator::calculate() |
| `src/core/channels.rs` | L18-25 | SpreadOpportunity struct |
| `src/core/execution.rs` | L345-372 | DeltaNeutralExecutor::new() |
| `6-1-main-runtime-integration.md` | Full | Previous story patterns |

---

### Testing Strategy

**Baseline Tests:** 241 tests passing (from Story 6.1)

**Story 6.2 Testing Approach:**

**Unit Tests (NEW):**
```rust
// tests in src/core/monitoring.rs

#[tokio::test]
async fn test_monitoring_task_shutdown() {
    // Create mock adapters and channels
    // Spawn monitoring_task
    // Send shutdown signal
    // Assert task terminates cleanly
}

#[tokio::test]
async fn test_spread_opportunity_sent_when_threshold_exceeded() {
    // Create mock adapters with mock orderbooks
    // Set threshold to 0.01%
    // Verify opportunity_rx receives SpreadOpportunity
}
```

**Manual Validation:**

```bash
# 1. Ensure .env and config.yaml configured
# 2. Temporarily set spread_entry to 0.01% in config.yaml for testing

# 3. Build
cargo build

# 4. Clippy
cargo clippy --all-targets -- -D warnings

# 5. Unit tests
cargo test

# 6. Manual runtime test
cargo run

# Expected logs:
# [INFO] 🚀 HFT Arbitrage Bot MVP starting...
# [INFO] Bot runtime started
# Monitoring task started
# Execution task started
# [DEBUG] Spread: 0.1234%  (continuous)
# [INFO] Spread opportunity detected: spread=0.35%, threshold=0.01%
# [TRADE] Entry executed: spread=0.35%
# [STATE] Position saved: pair=BTC-PERP

# 7. Ctrl+C - verify both tasks shutdown
# Monitoring task shutting down
# Execution task shutting down
# [SHUTDOWN] Clean exit
```

---

### Common LLM Mistakes to PREVENT (Story 6.2 Specific)

#### 🚫 **Mistake #1: Not Cloning Orderbook from get_orderbook()**

**Bad:**
```rust
// ❌ get_orderbook returns reference, can't hold across await
let vest_ob = vest.lock().await.get_orderbook(&symbol);
// ... do stuff with vest_ob after releasing lock
```

**Correct:**
```rust
// ✅ Clone immediately or use within lock scope
let vest_ob = {
    let guard = vest.lock().await;
    guard.get_orderbook(&symbol).cloned()
};
if let Some(ob) = vest_ob {
    // Safe to use cloned orderbook
}
```

---

#### 🚫 **Mistake #2: Not Using tokio::select! for Shutdown**

**Bad:**
```rust
// ❌ Can't shutdown - infinite loop
loop {
    poll_interval.tick().await;
    // process
}
```

**Correct:**
```rust
// ✅ Shutdown-aware loop
loop {
    tokio::select! {
        _ = shutdown_rx.recv() => { break; }
        _ = poll_interval.tick() => { /* process */ }
    }
}
```

---

#### 🚫 **Mistake #3: Forgetting to Clone Arc Before Spawning**

**Bad:**
```rust
// ❌ Moved into spawn, can't use vest later
tokio::spawn(async move {
    monitoring_task(vest, paradex, ...).await;
});
// vest is now moved!
```

**Correct:**
```rust
// ✅ Clone before spawn
let vest_clone = vest.clone();
let paradex_clone = paradex.clone();
tokio::spawn(async move {
    monitoring_task(vest_clone, paradex_clone, ...).await;
});
// Original vest/paradex still available
```

---

#### 🚫 **Mistake #4: Blocking on Channel Send**

**Bad:**
```rust
// ❌ If channel full, blocks monitoring
opportunity_tx.send(opportunity).await.unwrap();
```

**Correct:**
```rust
// ✅ Non-blocking with warning
match opportunity_tx.try_send(opportunity) {
    Ok(_) => { /* sent */ }
    Err(mpsc::error::TrySendError::Full(_)) => {
        warn!("Opportunity channel full, dropping opportunity");
    }
    Err(e) => {
        error!("Failed to send opportunity: {}", e);
    }
}
```

---

### Expected Behavior After Story 6.2

**Scenario: Automatic Trade Execution**

```bash
$ cargo run

[INFO] 🚀 HFT Arbitrage Bot MVP starting...
[INFO] 📁 Loading configuration from config.yaml...
[CONFIG] Loaded pairs: ["BTC-PERP"]
[INFO] Vest adapter initialized
[INFO] Paradex adapter initialized
[INFO] Connected to Vest
[INFO] Connected to Paradex
[STATE] Restored 0 positions from database
[INFO] Subscribed to orderbooks: BTC-PERP, BTC-USD-PERP
[INFO] Bot runtime started
Monitoring task started
Execution task started
[SHUTDOWN] SIGINT handler registered

# ... continuous polling ...
[DEBUG] Spread: 0.0523%
[DEBUG] Spread: 0.0891%
[DEBUG] Spread: 0.1234%

# Spread exceeds threshold (0.30%)
[INFO] Spread opportunity detected: spread=0.3542%, threshold=0.30%
Processing spread opportunity
[TRADE] Entry executed: spread=0.3542%, long=vest, short=paradex
[STATE] Position saved: pair=BTC-PERP, entry_spread=0.3542%

# Continues monitoring for next opportunity...
```

**Scenario: Graceful Shutdown**

```bash
# User presses Ctrl+C

[SHUTDOWN] Graceful shutdown initiated
Monitoring task shutting down
Monitoring task stopped
Execution task shutting down
Execution task stopped
[SHUTDOWN] Clean exit, no pending orders
[SHUTDOWN] Disconnected from exchanges
[SHUTDOWN] Clean exit
# Exit code: 0
```

---

### FR Coverage

Story 6.2 **implements automatic execution loop** for all FRs:

**Epic 1 - Market Data (active monitoring):**
- FR3: Calcul spread via SpreadCalculator (Task 1.3)
- FR4: Détection dépassement seuil (Task 1.4)

**Epic 2 - Execution (automatic trigger):**
- FR7: Exécution delta-neutral automatique (Task 3)

**Epic 3 - State Persistence:**
- FR10: Sauvegarde position après trade (Task 4.2)

**Epic 6.2 Specific Contribution:**
- **Automatic Pipeline**: monitoring → detection → execution → persistence
- **No Manual Intervention**: fully autonomous trading loop

---

### NFR Alignment

**NFR Coverage via Implementation:**

- **NFR1 (Spread calculation <2ms):** SpreadCalculator already optimized
- **NFR2 (Detection-to-order <500ms):** Channel direct, executor uses tokio::join!
- **NFR10 (State recovery):** Positions saved to Supabase after each trade
- **NFR11 (Graceful shutdown):** Both tasks respond to shutdown signal

---

### Git Intelligence (Recent Commits)

```
6269f33 fix(code-review): Story 6.1 - Fix review issues
11f6532 feat(story-6.1): Main Runtime Integration - complete implementation
4b73be2 feat(resilience): Story 4.6 - Protection contre les ordres orphelins
6f9f65e feat(resilience): Story 4.5 - Arrêt propre sur SIGINT
c6356ee feat(resilience): Story 4.4 - Reconnexion automatique WebSocket
```

**Recommended commit message for Story 6.2:**

```
feat(automation): Story 6.2 - Automatic Delta-Neutral Execution

- Create monitoring_task for continuous orderbook polling (100ms)
- Calculate spreads using SpreadCalculator
- Detect threshold crossings and emit SpreadOpportunity
- Spawn execution_task with DeltaNeutralExecutor
- Connect monitoring → execution via mpsc channel
- Save positions to Supabase after successful trades

Bot now trades autonomously when spread exceeds entry threshold.
Ready for Story 6.3 (automatic position monitoring & exit).
```

---

### Epic 6 Integration Notes

**Story 6.2 Deliverables:**
- ✅ Monitoring task polling orderbooks continuously
- ✅ Spread calculation and threshold detection
- ✅ Automatic execution pipeline via channels
- ✅ Position persistence after successful trades

**Story 6.3 Requirements (Automatic Exit):**
- Monitor open positions for exit conditions
- Close positions when spread ≤ exit threshold
- Update Supabase on position close

**Story 6.5 Requirements (Integration Test):**
- Automated test covering full cycle (detect → execute → persist → exit)
- Testnet or mocked exchanges

**Story 6.2 Success Criteria → Next Story Readiness:**
- ✅ Bot detects opportunities automatically
- ✅ Bot executes trades without intervention
- ✅ Positions persisted to Supabase
- ➡️ **Next:** Story 6.3 implements automatic exit monitoring

---

### References

- [Source: epics.md#Story-6.2] Story 6.2 requirements (automatic execution)
- [Source: src/core/runtime.rs#L23-76] execution_task pattern
- [Source: src/core/spread.rs#L68-131] SpreadCalculator::calculate()
- [Source: src/core/channels.rs#L18-25] SpreadOpportunity struct
- [Source: src/core/execution.rs#L345-372] DeltaNeutralExecutor::new()
- [Source: src/main.rs#L107-163] TODOs for Story 6.2 integration
- [Source: 6-1-main-runtime-integration.md] Previous story patterns
- [Source: config.yaml] Bot config structure (spread_entry threshold)
- [Source: sprint-status.yaml#L136-142] Epic 6 status

---

## Dev Agent Record

### Agent Model Used

{{agent_model_name_version}}

### Debug Log References

### Completion Notes List

### File List
