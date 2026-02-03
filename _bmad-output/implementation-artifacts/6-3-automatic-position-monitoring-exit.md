# Story 6.3: Automatic Position Monitoring & Exit

Status: review

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **opérateur**,
I want que les positions se ferment automatiquement quand spread ≤ exit threshold,
So that je capture les profits sans monitoring manuel.

## Acceptance Criteria

1. **Given** une position delta-neutral ouverte  
   **When** spread ≤ `spread_exit` threshold configuré  
   **Then** position est **automatiquement fermée**  
   **And** les deux legs sont fermés simultanément  
   **And** Supabase est mis à jour (Story 3.4)  
   **And** un log `[TRADE] Auto-closed: spread=X%` est émis

2. **Given** des positions restaurées de Supabase au démarrage (Story 3.3)  
   **When** le bot reprend le monitoring  
   **Then** il tracke les conditions de sortie pour ces positions

3. **Given** le monitoring actif pour positions ouvertes  
   **When** les orderbooks sont mis à jour  
   **Then** le spread de sortie est calculé en continu  
   **And** le calcul s'exécute en <2ms (NFR1)

4. **Given** une position fermée avec succès  
   **When** le close est confirmé  
   **Then** la position est marquée `status: Closed` dans Supabase  
   **And** le log `[STATE] Position closed` est émis

## Tasks / Subtasks

### 🎯 TASK OVERVIEW: Automatic Position Exit Pipeline

**Ce qui existe déjà (Stories 6.1 + 6.2 + Epic 3):**
- ✅ monitoring_task polling orderbooks (src/core/monitoring.rs)
- ✅ execution_task executing trades (src/core/runtime.rs)
- ✅ SpreadCalculator calculant spread entry (src/core/spread.rs)
- ✅ DeltaNeutralExecutor pour exécution simultanée (src/core/execution.rs)
- ✅ StateManager avec load/save/update/remove (src/core/state.rs)
- ✅ PositionState avec status Open/Closed (src/core/state.rs)
- ✅ spread_exit configuré dans config.yaml
- ✅ Shutdown broadcast pattern (Story 4.5)
- ✅ Positions restaurées dans main.rs au démarrage (L195-212)

**Ce qui manque (Story 6.3):**
- Un position_monitoring_task qui tracke les positions ouvertes
- Détection du spread ≤ exit threshold pour chaque position
- Exécution des ordres de close (reduce_only) sur les deux legs
- Mise à jour du status dans Supabase après close
- Intégration des positions restaurées dans le monitoring

---

- [x] **Task 1**: Créer le position_monitoring_task (AC: Position Exit Monitoring)
  - [ ] Subtask 1.1: Créer `src/core/position_monitor.rs` avec `position_monitoring_task()`
    - Signature: `async fn position_monitoring_task<V, P>(vest: Arc<Mutex<V>>, paradex: Arc<Mutex<P>>, state_manager: Arc<StateManager>, executor: Arc<DeltaNeutralExecutor<V, P>>, config: PositionMonitoringConfig, shutdown_rx: broadcast::Receiver<()>)`
    - Pattern: identique à monitoring_task (polling interval 100ms, select! avec shutdown)
  - [ ] Subtask 1.2: Définir `PositionMonitoringConfig`
    - Champs: pair, spread_exit, vest_symbol, paradex_symbol
  - [ ] Subtask 1.3: Charger les positions initiales depuis state_manager
    - `let mut open_positions: Vec<PositionState> = state_manager.load_positions().await?`
    - Log: `[STATE] Monitoring N positions for exit conditions`

- [x] **Task 2**: Implémenter la détection de condition de sortie (AC: Exit Detection)
  - [ ] Subtask 2.1: Calculer le spread de sortie pour chaque position ouverte
    - Récupérer orderbooks des deux exchanges (pattern de monitoring_task)
    - Calculer spread avec SpreadCalculator
    - Log debug: `Spread for position {id}: {spread}%`
  - [ ] Subtask 2.2: Détecter le dépassement du seuil exit
    - Condition: `spread <= config.spread_exit`
    - Log: `[TRADE] Exit condition met: spread={spread}%, threshold={exit}%`
  - [ ] Subtask 2.3: Gérer la direction du spread
    - Pour position existante: calculer spread inverse à l'entrée
    - Si entry était A>B, exit est quand B>A diminue

- [x] **Task 3**: Implémenter la fermeture automatique des positions (AC: Auto-Close)
  - [ ] Subtask 3.1: Créer la logique de close delta-neutral
    - Utiliser les méthodes d'adapter existantes avec `reduce_only: true`
    - Vest: close long = SELL reduce_only
    - Paradex: close short = BUY reduce_only
  - [ ] Subtask 3.2: Exécuter les closes simultanément via tokio::join!
    - Pattern identique à DeltaNeutralExecutor::execute_delta_neutral
    - Gérer erreurs: si un leg échoue, retry (pattern Story 2.4)
  - [ ] Subtask 3.3: Logger le résultat
    - Succès: `[TRADE] Auto-closed: spread=X%`
    - Échec partiel: `[TRADE] Close partially failed` avec détails

- [x] **Task 4**: Mettre à jour Supabase après close (AC: State Update)
  - [ ] Subtask 4.1: Appeler `state_manager.update_position()` avec status Closed
    - Update: `PositionUpdate { status: Some(PositionStatus::Closed), remaining_size: Some(0.0) }`
  - [ ] Subtask 4.2: Retirer la position du monitoring local
    - Logique: `open_positions.retain(|p| p.id != closed_position.id)`
  - [ ] Subtask 4.3: Logger le résultat
    - Succès: `[STATE] Position closed`
    - Échec: `[STATE] Failed to update position status (trading continues)`

- [x] **Task 5**: Intégrer le task dans main.rs (AC: Integration)
  - [ ] Subtask 5.1: Importer position_monitoring_task et PositionMonitoringConfig
    - `use hft_bot::core::position_monitor::{position_monitoring_task, PositionMonitoringConfig};`
    - Export dans `src/core/mod.rs`
  - [ ] Subtask 5.2: Créer PositionMonitoringConfig depuis BotConfig
    - `spread_exit: config.bots[0].spread_exit`
    - Réutiliser vest_symbol, paradex_symbol existants
  - [ ] Subtask 5.3: Spawn position_monitoring_task
    - Créer nouvelles instances d'adapters (pattern Story 6.2 Task 3)
    - Clone state_manager pour partage
    - `tokio::spawn(position_monitoring_task(...))`

- [x] **Task 6**: Considérer les positions restaurées (AC: Restored Positions)
  - [ ] Subtask 6.1: Passer les positions restaurées au position_monitoring_task
    - Les positions sont déjà chargées dans main.rs (L195-212)
    - Soit passer directement, soit laisser le task les charger lui-même
  - [ ] Subtask 6.2: Fusionner nouvelles positions et positions restaurées
    - Quand execution_task crée une nouvelle position → notifier position_monitoring_task
    - Option A: Channel mpsc<PositionState> entre execution_task et position_monitor
    - Option B: position_monitor recharge depuis Supabase périodiquement
    - **Recommandé:** Option A (channel) pour réactivité

- [x] **Task 7**: Tests et validation (AC: All Tests Pass)
  - [ ] Subtask 7.1: `cargo build` - code compile sans warnings
  - [ ] Subtask 7.2: `cargo clippy --all-targets -- -D warnings` - 0 warnings
  - [ ] Subtask 7.3: `cargo test` - baseline tests passent (244+ tests)
  - [ ] Subtask 7.4: Ajouter tests unitaires pour position_monitoring_task
    - Test: shutdown proprement sur signal
    - Test: position fermée quand spread <= exit
    - Test: position non fermée quand spread > exit
    - Test: Supabase update appelé après close

---

## Dev Notes

### 🎯 STORY FOCUS: Automatic Exit Pipeline (Epic 6.3)

**Mission:** Compléter le cycle de trading automatique en ajoutant la fermeture automatique des positions quand le spread atteint le seuil de sortie.

**Key Integration Points:**
1. **position_monitoring_task** → poll orderbooks → calculer exit spread → détecter condition
2. **Close execution** → fermer les deux legs simultanément (reduce_only)
3. **StateManager** → update position status to Closed

---

### Previous Story Intelligence (Story 6.2)

#### **Story 6.2 — Automatic Delta-Neutral Execution**

**Learnings:**
- ✅ monitoring_task pattern: polling 100ms, select! avec shutdown
- ✅ SpreadCalculator usage: `calculator.calculate(&vest_ob, &paradex_ob)`
- ✅ StateManager integration: save_position après trade réussi
- ✅ Séparation adapters: nouvelles instances pour execution vs monitoring
- ✅ Channel pattern: mpsc<SpreadOpportunity> pour communication

**Common LLM Mistakes from Story 6.2:**
- ⚠️ Not cloning orderbook from get_orderbook() before releasing lock
- ⚠️ Blocking channel send instead of try_send()
- ⚠️ Forgetting `.clone()` for Arc before spawning
- ⚠️ Not using `.subscribe()` for shutdown receiver

---

### Architecture Compliance — Position Monitoring Pattern

#### **monitoring_task Pattern (src/core/monitoring.rs L54-144)**

```rust
pub async fn monitoring_task<V, P>(
    vest: Arc<Mutex<V>>,
    paradex: Arc<Mutex<P>>,
    opportunity_tx: mpsc::Sender<SpreadOpportunity>,
    vest_symbol: String,
    paradex_symbol: String,
    config: MonitoringConfig,
    mut shutdown_rx: broadcast::Receiver<()>,
) where
    V: ExchangeAdapter + Send,
    P: ExchangeAdapter + Send,
{
    info!("Monitoring task started");
    
    let calculator = SpreadCalculator::new("vest", "paradex");
    let mut poll_interval = interval(Duration::from_millis(POLL_INTERVAL_MS));
    
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Monitoring task shutting down");
                break;
            }
            _ = poll_interval.tick() => {
                // ... poll and calculate
            }
        }
    }
}
```

**→ position_monitoring_task doit suivre exactement ce même pattern.**

---

#### **StateManager Update (src/core/state.rs L452-512)**

```rust
// Update position status
let update = PositionUpdate {
    remaining_size: Some(0.0),
    status: Some(PositionStatus::Closed),
};
state_manager.update_position(position.id, update).await?;
```

---

#### **DeltaNeutralExecutor Close Pattern (src/core/execution.rs)**

Pour fermer une position, utiliser le même executor mais avec:
- `reduce_only: true` dans OrderRequest
- `side: SELL` pour long leg, `side: BUY` pour short leg

```rust
// Close long position on Vest
let close_long = OrderRequest {
    symbol: position.long_symbol.clone(),
    side: OrderSide::Sell,
    quantity: position.long_size,
    reduce_only: true,
    ..
};

// Close short position on Paradex  
let close_short = OrderRequest {
    symbol: position.short_symbol.clone(),
    side: OrderSide::Buy,
    quantity: position.short_size,
    reduce_only: true,
    ..
};
```

---

### Library/Framework Requirements — No New Dependencies

**Existing dependencies suffisent:**
- ✅ `tokio` (async, sync, time::interval)
- ✅ `tracing` (logging)
- ✅ `uuid` pour position IDs
- ✅ StateManager déjà initialisé dans main.rs

---

### File Structure Requirements

**Files to CREATE:**

| File | Type | Approx LOC | Description |
|------|------|------------|-------------|
| `src/core/position_monitor.rs` | **NEW** | ~150-180 | position_monitoring_task function |

**Files to MODIFY:**

| File | Type | Approx LOC Change | Description |
|------|------|-------------------|-------------|
| `src/core/mod.rs` | **MODIFY** | +1 | Export position_monitor module |
| `src/main.rs` | **MODIFY** | +25-35 | Spawn position_monitoring_task |

**Files to REFERENCE (Read-Only):**

| File | Lines | Reason |
|------|-------|--------|
| `src/core/monitoring.rs` | L54-144 | monitoring_task pattern (clone for position_monitor) |
| `src/core/runtime.rs` | L27-125 | execution_task with StateManager |
| `src/core/state.rs` | L452-512 | update_position API |
| `src/core/execution.rs` | L100-200 | DeltaNeutralExecutor close pattern |
| `6-2-automatic-delta-neutral-execution.md` | Full | Previous story patterns |
| `config.yaml` | L7 | spread_exit threshold (0.05) |

---

### Testing Strategy

**Baseline Tests:** 244 tests passing (from Story 6.2)

**Story 6.3 Testing Approach:**

**Unit Tests (NEW):**
```rust
// tests in src/core/position_monitor.rs

#[tokio::test]
async fn test_position_monitoring_task_shutdown() {
    // Create mock adapters and state manager
    // Spawn position_monitoring_task
    // Send shutdown signal
    // Assert task terminates cleanly
}

#[tokio::test]
async fn test_position_closed_when_exit_threshold_met() {
    // Create position with entry spread 0.30%
    // Set exit threshold 0.05%
    // Mock orderbooks with spread <= 0.05%
    // Verify close is triggered
    // Verify StateManager.update_position called
}

#[tokio::test]
async fn test_position_not_closed_when_above_exit_threshold() {
    // Create position
    // Mock orderbooks with spread > exit threshold
    // Verify position remains open
}
```

**Manual Validation:**

```bash
# 1. Ensure .env and config.yaml configured with spread_exit: 0.05

# 2. Build
cargo build

# 3. Clippy
cargo clippy --all-targets -- -D warnings

# 4. Unit tests
cargo test

# 5. Manual runtime test
cargo run

# Expected logs:
# [INFO] 🚀 HFT Arbitrage Bot MVP starting...
# [STATE] Restored N positions from database
# [INFO] Bot runtime started
# Monitoring task started
# Execution task started
# Position monitoring task started
# [STATE] Monitoring 1 positions for exit conditions

# When spread drops below exit threshold:
# [TRADE] Exit condition met: spread=0.04%, threshold=0.05%
# [TRADE] Auto-closed: spread=0.04%
# [STATE] Position closed

# 6. Ctrl+C - verify all tasks shutdown
# Position monitoring task shutting down
# Monitoring task shutting down
# Execution task shutting down
```

---

### Common LLM Mistakes to PREVENT (Story 6.3 Specific)

#### 🚫 **Mistake #1: Not Using reduce_only for Close Orders**

**Bad:**
```rust
// ❌ Opens new position instead of closing
let order = OrderRequest {
    side: OrderSide::Sell,
    quantity: position.long_size,
    // Missing reduce_only: true
};
```

**Correct:**
```rust
// ✅ Closes existing position
let order = OrderRequest {
    side: OrderSide::Sell,
    quantity: position.long_size,
    reduce_only: true,  // Critical!
};
```

---

#### 🚫 **Mistake #2: Not Handling Partial Close**

**Consideration:**
Si un leg close mais l'autre échoue, il faut:
1. Retry l'autre leg (pattern Story 2.4)
2. Ou auto-close le leg qui a réussi (pattern Story 2.5)

```rust
// Handle partial close scenario
match (long_result, short_result) {
    (Ok(_), Err(e)) => {
        // Short leg failed - retry or compensate
        warn!("Short leg close failed: {}", e);
        // Retry logic...
    }
    (Err(e), Ok(_)) => {
        // Long leg failed - retry or compensate
        warn!("Long leg close failed: {}", e);
        // Retry logic...
    }
    _ => { /* Both succeeded or both failed */ }
}
```

---

#### 🚫 **Mistake #3: Calculating Exit Spread Incorrectly**

**Context:**
Le spread d'entrée peut être A>B ou B>A. Le spread de sortie doit être calculé correctement par rapport à la direction d'entrée.

**Correct Approach:**
```rust
// For exit, we want the spread to converge to zero
// If entry was Vest > Paradex (buying on Paradex, selling on Vest)
// Exit is when Vest <= Paradex + exit_threshold

// Simply: use the absolute spread value
let exit_spread = spread_result.spread_pct.abs();
if exit_spread <= config.spread_exit {
    // Exit condition met
}
```

---

#### 🚫 **Mistake #4: Not Synchronizing New Positions**

**Challenge:**
Quand execution_task crée une nouvelle position, position_monitoring_task doit en être informé.

**Solution Recommandée: Channel**
```rust
// In main.rs - create channel
let (new_position_tx, new_position_rx) = mpsc::channel::<PositionState>(10);

// Pass tx to execution_task
execution_task(..., new_position_tx, ...).await

// Pass rx to position_monitoring_task  
position_monitoring_task(..., new_position_rx, ...).await

// In position_monitoring_task
tokio::select! {
    _ = shutdown_rx.recv() => { break; }
    _ = poll_interval.tick() => { /* check exit conditions */ }
    Some(new_pos) = new_position_rx.recv() => {
        open_positions.push(new_pos);
        info!("New position added to monitoring");
    }
}
```

---

### Expected Behavior After Story 6.3

**Scenario: Full Trading Cycle**

```bash
$ cargo run

[INFO] 🚀 HFT Arbitrage Bot MVP starting...
[INFO] 📁 Loading configuration from config.yaml...
[CONFIG] Loaded pairs: ["BTC-PERP"]
[STATE] Restored 0 positions from database
[INFO] Bot runtime started
Monitoring task started
Execution task started
Position monitoring task started
[STATE] Monitoring 0 positions for exit conditions

# ... spread exceeds entry threshold (0.30%) ...
[INFO] Spread opportunity detected: spread=0.35%, threshold=0.30%
[TRADE] Entry executed: spread=0.35%, long=vest, short=paradex
[STATE] Position saved: pair=BTC-PERP, entry_spread=0.35%
[INFO] New position added to exit monitoring

# ... spread drops to exit threshold (0.05%) ...
[DEBUG] Spread for position abc123: 0.04%
[TRADE] Exit condition met: spread=0.04%, threshold=0.05%
[TRADE] Auto-closed: spread=0.04%
[STATE] Position closed: pair=BTC-PERP

# ... continues monitoring for new opportunities ...
```

**Scenario: Restored Positions**

```bash
$ cargo run

[INFO] 🚀 HFT Arbitrage Bot MVP starting...
[STATE] Restored 2 positions from database
Position monitoring task started
[STATE] Monitoring 2 positions for exit conditions

# Immediately check exit conditions for restored positions
[DEBUG] Spread for position abc123: 0.12%
[DEBUG] Spread for position def456: 0.03%
[TRADE] Exit condition met: spread=0.03%, threshold=0.05%
[TRADE] Auto-closed: spread=0.03%
[STATE] Position closed: pair=BTC-PERP
```

---

### FR Coverage

Story 6.3 **complètes the automatic trading loop**:

**From Epic 6.3 Requirements:**
- AC1: Fermeture automatique quand spread ≤ exit threshold ✓
- AC2: Suivi des positions restaurées ✓
- AC3: Mise à jour Supabase après close ✓

**Epic Integration:**
- FR7: Exécution delta-neutral (close = reverse execution)
- FR10-12: State persistence (update position status)

---

### NFR Alignment

**NFR Coverage via Implementation:**

- **NFR1 (Spread calculation <2ms):** SpreadCalculator already optimized
- **NFR2 (Detection-to-order <500ms):** Close triggered immediately on detection
- **NFR10 (State recovery):** Restored positions monitored for exit
- **NFR14 (Supabase stable):** StateManager handles HTTP errors gracefully

---

### Git Intelligence (Recent Commits)

```
9b2136f fix(6.2): implement StateManager persistence + log format fixes
6269f33 fix(code-review): Story 6.1 - Fix review issues
11f6532 feat(story-6.1): Main Runtime Integration - complete implementation
```

**Recommended commit message for Story 6.3:**

```
feat(automation): Story 6.3 - Automatic Position Monitoring & Exit

- Create position_monitoring_task for exit condition detection
- Poll orderbooks and calculate exit spreads for open positions
- Execute simultaneous close orders when spread <= exit_threshold
- Update Supabase position status to Closed after successful close
- Integrate restored positions into exit monitoring
- Add channel for new position synchronization

Bot now completes full trading cycle: entry → monitoring → exit.
Ready for Story 6.5 (end-to-end integration test).
```

---

### Design Decisions

**Decision 1: Separate position_monitoring_task vs Extending monitoring_task**

**Choice:** Separate task
**Rationale:**
- Single Responsibility: entry detection vs exit detection
- Different data flows: entry detects opportunity → execution, exit monitors positions → close
- Easier testing and debugging  
- Follows existing pattern (monitoring_task, execution_task)

**Decision 2: Channel vs Periodic Supabase Reload for New Positions**

**Choice:** Channel (`mpsc<PositionState>`)
**Rationale:**
- Immediate notification when new position created
- No unnecessary Supabase requests
- Lower latency for position monitoring
- Follows existing channel pattern in codebase

**Decision 3: SharedPositions vs ChannelSynchronization**

**Alternative:** Use `Arc<RwLock<Vec<PositionState>>>` shared between tasks
**Chosen:** Channel-based synchronization
**Rationale:**
- Avoids lock contention
- Clearer ownership semantics
- Follows message-passing pattern preferred in Tokio

---

### Epic 6 Integration Notes

**Story 6.3 Deliverables:**
- ✅ Position monitoring task for exit conditions
- ✅ Automatic close when spread ≤ exit threshold
- ✅ Supabase update on position close
- ✅ Restored positions integrated into monitoring

**Story 6.5 Requirements (Integration Test):**
- Full cycle test: entry → persist → exit → verify closed
- Testnet or mocked exchanges
- Verify state consistency end-to-end

**Story 6.3 Success Criteria → MVP Feature Complete:**
- ✅ Bot enters positions automatically (Story 6.2)
- ✅ Bot exits positions automatically (Story 6.3)
- ✅ State persisted throughout lifecycle (Stories 3.1-3.4)
- ➡️ **Next:** Story 6.5 validates full cycle via automated tests

---

### References

- [Source: epics.md#Story-6.3] Story 6.3 requirements (automatic exit)
- [Source: src/core/monitoring.rs#L54-144] monitoring_task pattern
- [Source: src/core/runtime.rs#L27-125] execution_task with StateManager
- [Source: src/core/state.rs#L452-512] update_position API
- [Source: src/core/execution.rs] DeltaNeutralExecutor patterns
- [Source: config.yaml#L7] spread_exit threshold (0.05)
- [Source: 6-2-automatic-delta-neutral-execution.md] Previous story patterns
- [Source: sprint-status.yaml#L140] Story status

---

## Dev Agent Record

### Agent Model Used

Gemini 2.5 Pro (Antigravity)

### Debug Log References

### Completion Notes List

- Code review identified and fixed CRITICAL exit threshold logic bug (>= was <=)
- Fixed log format issues to match AC requirements ([TRADE], [STATE] prefixes)
- Build, clippy (0 warnings), tests (250 passed) all pass

### File List

| File | Lines Changed | Description |
|------|---------------|-------------|
| src/core/position_monitor.rs | L1-705 (NEW) | Position monitoring task with exit detection and auto-close |
| src/core/mod.rs | L22, L63-64 | Added position_monitor module export |
| src/core/runtime.rs | L32, L100-128 | Added new_position_tx channel for position sync |
| src/main.rs | L24, L203-241 | Spawn position_monitoring_task with channel integration |

