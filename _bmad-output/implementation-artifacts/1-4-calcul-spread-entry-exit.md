# Story 1.4: Calcul de Spread Entry/Exit

Status: done

<!-- Note: Epic 1 Story 4 - Wire SpreadCalculator to consume orderbook channel and emit spread opportunities. -->

## Story

As a **opérateur**,
I want que le bot calcule les spreads entry et exit,
So that je puisse voir les opportunités d'arbitrage.

## Acceptance Criteria

1. **Given** des orderbooks disponibles pour les deux exchanges
   **When** les orderbooks sont mis à jour
   **Then** le spread entry (Vest ask - Paradex bid) est calculé
   **And** le spread exit (Paradex ask - Vest bid) est calculé
   **And** le calcul s'exécute en < 2ms (NFR1)
   **And** les spreads sont émis vers le channel correspondant

## Tasks / Subtasks

- [x] **Task 1**: Créer la task SpreadMonitor (AC: #1)
  - [x] Subtask 1.1: Créer `SpreadMonitor` struct dans `core/spread.rs`
  - [x] Subtask 1.2: Implémenter `async fn run(&mut self)` qui consomme `orderbook_rx`
  - [x] Subtask 1.3: Maintenir un cache `Option<Orderbook>` par exchange (vest/paradex)
  - [x] Subtask 1.4: Appeler `SpreadCalculator.calculate_dual_spreads()` quand les deux orderbooks sont présents

- [x] **Task 2**: Intégrer avec ChannelBundle (AC: #1)
  - [x] Subtask 2.1: Le monitor reçoit `orderbook_rx` depuis `ChannelBundle`
  - [x] Subtask 2.2: Le monitor émet `SpreadOpportunity` vers `opportunity_tx`
  - [x] Subtask 2.3: Passer `dex_a="vest"`, `dex_b="paradex"` au SpreadCalculator

- [x] **Task 3**: Ajouter logging structuré (AC: #1)
  - [x] Subtask 3.1: Ajouter `debug!(pair, entry_spread, exit_spread, "Spread calculated")` à chaque calcul
  - [x] Subtask 3.2: Ajouter `info!(pair, spread=%, "Spread opportunity detected")` si spread > 0

- [x] **Task 4**: Test de performance NFR1 (AC: #1)
  - [x] Subtask 4.1: Écrire `test_spread_calculation_performance_2ms`
  - [x] Subtask 4.2: Test avec orderbooks de 100+ levels chaque
  - [x] Subtask 4.3: Utiliser `std::time::Instant` pour mesurer le temps

- [x] **Task 5**: Tests unitaires et d'intégration (AC: #1)
  - [x] Subtask 5.1: `test_spread_monitor_processes_orderbook_update`
  - [x] Subtask 5.2: `test_spread_opportunity_emitted_on_spread_change`
  - [x] Subtask 5.3: `test_spread_monitor_handles_missing_orderbook`

- [x] **Task 6**: Validation finale (AC: #1)
  - [x] Subtask 6.1: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 6.2: `cargo test` tous les tests passent (229 tests)
  - [x] Subtask 6.3: Vérifier que le calcul respecte NFR1 (< 2ms verified)

## Dev Notes

### Contexte Brownfield — Code Existant

> ⚠️ **CRITICAL**: Ce projet est brownfield avec ~8,900 lignes existantes. Le SpreadCalculator existe déjà avec toutes les méthodes nécessaires.

**L'objectif est de CRÉER le SpreadMonitor qui fait le pont entre orderbook channel et spread calculation.**

### Analyse du Code Existant

| Composant | Status | Fichier | Lignes |
|-----------|--------|---------|--------|
| `SpreadCalculator` | ✅ Existe | `core/spread.rs` | 68-220 |
| `SpreadCalculator.calculate_entry_spread()` | ✅ Existe | `core/spread.rs` | 152-172 |
| `SpreadCalculator.calculate_exit_spread()` | ✅ Existe | `core/spread.rs` | 174-194 |
| `SpreadCalculator.calculate_dual_spreads()` | ✅ Existe | `core/spread.rs` | 196-220 |
| `SpreadOpportunity` | ✅ Existe | `core/channels.rs` | 17-25 |
| `ChannelBundle.opportunity_tx/rx` | ✅ Existe | `core/channels.rs` | 31-32 |
| `ChannelBundle.orderbook_tx/rx` | ✅ Existe | `core/channels.rs` | 35-36 |
| `SpreadDirection` | ✅ Existe | `core/spread.rs` | via channels.rs re-export |
| `SpreadMonitor` | ❌ À créer | `core/spread.rs` ou `core/monitor.rs` | — |

### Architecture Guardrails

**Fichiers à modifier/créer :**
- `src/core/spread.rs` — ajouter `SpreadMonitor` struct et `run()` method OU
- `src/core/monitor.rs` [NEW] — si préféré, créer nouveau fichier pour séparation

**Fichiers à NE PAS modifier :**
- `src/core/channels.rs` — channels déjà configurés, ne pas changer
- `src/adapters/` — adapters émettent déjà vers `orderbook_tx` (Story 1.3)
- `src/config/` — pas de changements config pour cette story

**Patterns obligatoires (copiés de Stories précédentes) :**
```rust
// SpreadMonitor structure
pub struct SpreadMonitor {
    calculator: SpreadCalculator,
    vest_orderbook: Option<Orderbook>,
    paradex_orderbook: Option<Orderbook>,
    pair: String,
}

// Main loop pattern
impl SpreadMonitor {
    pub async fn run(
        &mut self,
        mut orderbook_rx: mpsc::Receiver<OrderbookUpdate>,
        opportunity_tx: mpsc::Sender<SpreadOpportunity>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), Error> {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                Some(update) = orderbook_rx.recv() => {
                    self.handle_orderbook_update(update, &opportunity_tx).await?;
                }
            }
        }
        Ok(())
    }
    
    fn handle_orderbook_update(&mut self, update: OrderbookUpdate, tx: &mpsc::Sender<SpreadOpportunity>) {
        // Store orderbook by exchange
        match update.exchange.as_str() {
            "vest" => self.vest_orderbook = Some(update.orderbook),
            "paradex" => self.paradex_orderbook = Some(update.orderbook),
            _ => {}
        }
        
        // Calculate spreads if both orderbooks present
        if let (Some(vest), Some(paradex)) = (&self.vest_orderbook, &self.paradex_orderbook) {
            if let Some((entry, exit)) = self.calculator.calculate_dual_spreads(vest, paradex) {
                debug!(pair = %self.pair, entry_spread = entry, exit_spread = exit, "Spread calculated");
                // Emit opportunity if significant
            }
        }
    }
}
```

### Flow de Données

```
┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│  Vest Adapter    │────▶│  orderbook_rx    │────▶│  SpreadMonitor   │
│  (Story 1.3)     │     │  (ChannelBundle) │     │  (Cette story)   │
└──────────────────┘     └──────────────────┘     └────────┬─────────┘
                                                           │
┌──────────────────┐     ┌──────────────────┐              │
│ Paradex Adapter  │────▶│  orderbook_rx    │──────────────┤
│  (Story 1.3)     │     │  (ChannelBundle) │              │
└──────────────────┘     └──────────────────┘              │
                                                           ▼
                         ┌──────────────────┐     ┌──────────────────┐
                         │  opportunity_rx   │◀────│  SpreadMonitor   │
                         │  (ChannelBundle) │     │  emits spreads   │
                         └──────────────────┘     └──────────────────┘
```

### Project Structure Notes

**Structure après cette story :**
```
src/core/
├── mod.rs           # Exports (ajouter SpreadMonitor si nouveau fichier)
├── spread.rs        # SpreadCalculator + SpreadMonitor (option A)
├── monitor.rs       # [NEW option B] SpreadMonitor séparé
├── channels.rs      # SpreadOpportunity, ChannelBundle
├── vwap.rs          # VWAP calculator
└── state.rs         # Supabase persistence
```

### Technical Requirements

**Performance NFR1 :**
- Calcul de spread < 2ms
- `SpreadCalculator.calculate_dual_spreads()` déjà optimisé
- Pas d'allocations dans le hot path
- `std::time::Instant` pour mesurer

**OrderbookUpdate reception :**
- Input: `OrderbookUpdate { symbol, exchange, orderbook }`
- Le champ `exchange` permet de distinguer vest/paradex
- Note: Vérifier que `OrderbookUpdate` a bien un champ `exchange`, sinon l'ajouter

**SpreadOpportunity emission :**
```rust
SpreadOpportunity {
    pair: "ETH-PERP".to_string(),
    dex_a: "vest".to_string(),
    dex_b: "paradex".to_string(),
    spread_percent: 0.35,  // entry ou exit spread
    direction: SpreadDirection::AOverB,
    detected_at_ms: now,
}
```

### Previous Story Intelligence

**Story 1.3 (Réception et Parsing des Orderbooks) :**
- `OrderbookUpdate` channel ajouté à `ChannelBundle`
- Adapters émettent vers `orderbook_tx` après parsing
- Log pattern: `debug!(pair = %symbol, "Orderbook updated")`
- Performance: parsing < 1ms vérifié

**Leçons apprises des stories précédentes :**
1. Utiliser `tracing` macros avec structured fields (pair, spread, exchange)
2. Pattern TDD: écrire tests, puis implémenter
3. Vérifier que `OrderbookUpdate` a le champ `exchange` — sinon corriger dans cette story

### ⚠️ Points d'Attention

1. **Champ `exchange` dans `OrderbookUpdate`**: Vérifier si présent, sinon l'ajouter à `types.rs`
2. **Cache orderbooks**: HashMap simple car single-pair MVP
3. **Quand émettre SpreadOpportunity**: À chaque mise à jour? Seulement si changement significatif?

### References

- [Source: architecture.md#Performance] — NFR1: Spread < 2ms
- [Source: architecture.md#Communication Patterns] — mpsc channels pour data flow
- [Source: epics.md#Story 1.4] — Acceptance criteria originaux
- [Source: spread.rs#SpreadCalculator] — Calculator existant (L68-220)
- [Source: spread.rs#calculate_dual_spreads] — Méthode dual spreads (L196-220)
- [Source: channels.rs#SpreadOpportunity] — Struct existante (L17-25)
- [Source: channels.rs#ChannelBundle] — Channels définis (L29-60)

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`) - 229 tests pass
- [x] `SpreadMonitor` créé avec `run()` loop
- [x] Consomme `orderbook_rx` et émet vers `opportunity_tx`
- [x] Spread entry/exit calculés via `SpreadCalculator.calculate_dual_spreads()`
- [x] Log `[DEBUG] Spread calculated` émis à chaque calcul
- [x] Test de performance NFR1 ajouté (< 2ms verified)

## Dev Agent Record

### Agent Model Used

Antigravity (Google DeepMind)

### Change Log

| Date | Change | Files |
|------|--------|-------|
| 2026-01-31 | Added `exchange` field to `OrderbookUpdate` | `adapters/types.rs` |
| 2026-01-31 | Created `SpreadMonitor` struct with orderbook caching | `core/spread.rs` |
| 2026-01-31 | Implemented `run()` async loop with tokio::select! | `core/spread.rs` |
| 2026-01-31 | Added structured logging with tracing macros | `core/spread.rs` |
| 2026-01-31 | Added 5 SpreadMonitor tests including NFR1 performance | `core/spread.rs` |
| 2026-01-31 | Exported `SpreadMonitor` from core module | `core/mod.rs` |
| 2026-01-31 | [CR-FIX] Added `exchange` field assertion to channels test | `core/channels.rs` |
| 2026-01-31 | [CR-FIX] SpreadMonitor now emits both entry AND exit opportunities | `core/spread.rs` |
| 2026-01-31 | [CR-FIX] Added direction=entry/exit to structured logs | `core/spread.rs` |
| 2026-01-31 | [CR-FIX] Exit spread uses SpreadDirection::BOverA | `core/spread.rs` |


### Completion Notes List

- Created `SpreadMonitor` struct with `new()`, `run()`, `handle_orderbook_update()`, and `has_both_orderbooks()` methods
- Uses `Option<Orderbook>` for vest/paradex caching instead of HashMap (simpler for single-pair MVP)
- Added `exchange` field to `OrderbookUpdate` to distinguish between orderbook sources
- Emits `SpreadOpportunity` when **entry_spread > 0** OR **exit_spread > 0** (dual opportunity emission after CR fix)
- All 229 tests pass, including 4 new SpreadMonitor async tests + 1 NFR1 performance test
- NFR1 performance verified: single calculation < 2ms, 1000 iterations < 200ms
- **Code Review Fixes Applied (2026-01-31):**
  - H1: Added missing `exchange` field assertion in channels.rs test
  - M1: SpreadMonitor now emits opportunities for BOTH entry and exit spreads
  - M2: Exit opportunities use `SpreadDirection::BOverA` (not hard-coded AOverB)
  - M3: Added `direction=entry/exit` field to structured opportunity logs

### File List

| File | Action | Description |
|------|--------|-------------|
| `src/adapters/types.rs` | Modified | Added `exchange: String` field to `OrderbookUpdate` |
| `src/core/spread.rs` | Modified | Added `SpreadMonitor`, `SpreadMonitorError`, and 5 new tests |
| `src/core/mod.rs` | Modified | Exported `SpreadMonitor` and `SpreadMonitorError` |
| `src/core/channels.rs` | Modified | Fixed test to include `exchange` field |
