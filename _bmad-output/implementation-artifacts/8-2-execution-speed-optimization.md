# Story 8.2: Execution Speed Optimization

Status: blocked

> [!WARNING]
> **BLOQUÉE**: Cette story est en attente de l'installation d'un VPS. La latence ADSL (~386ms) est incompressible côté code. Reprendre cette story après migration vers VPS cloud.

## Story

As a opérateur,
I want optimiser la vitesse d'exécution pour réduire le slippage,
So that je capture un spread plus proche de la target.

> [!IMPORTANT]
> **Story 8.1 Findings**: Le bottleneck est `order_to_confirm` (~386ms), représentant 99% du temps total. Le traitement interne du bot (detection → signal → order) est <1ms.
>
> **⚠️ CONTEXTE RÉSEAU**: L'opérateur est en **ADSL rural** (fin de ligne). La latence de 386ms est principalement due à la connexion internet, pas au traitement exchange. **Aucune optimisation logicielle ne peut réduire cette latence** — une connexion fibre/4G ou un VPS cloud serait la seule solution efficace.

## Acceptance Criteria

1. **Given** les insights de Story 8.1 (`order_to_confirm` = ~386ms bottleneck)
   **When** les optimisations sont implémentées
   **Then** le slippage moyen est réduit de manière mesurable
   **And** les améliorations sont vérifiables via les métriques `SlippageAnalysis` de Story 8.1

2. **Given** une opportunité de spread détectée
   **When** les ordres sont exécutés
   **Then** la latence `order_to_confirm` est réduite par rapport au baseline de 386ms
   **And** le spread capturé est plus proche du spread détecté

3. **Given** les mesures de slippage sur N trades
   **When** je compare avant/après optimisation
   **Then** une réduction quantifiable du slippage (en bps) est observée

## Tasks / Subtasks

### Strategy Selection (Basé sur Story 8.1 Findings)

**Bottleneck Analysis (from 8.1):**
- `detection_to_signal_ms`: 0-1ms ✅ Pas d'optimisation nécessaire
- `signal_to_order_ms`: 0ms ✅ Pas d'optimisation nécessaire
- `order_to_confirm_ms`: 385-386ms 🔴 **TARGET**

**Possible Optimizations (à évaluer):**

| Strategy | Impact Estimé | Complexité | Faisabilité |
|----------|---------------|------------|-------------|
| Optimistic Execution | Moyen | Faible | ✅ Implémentable |
| Pre-signed Orders | Haut | Moyen | ✅ Déjà partiellement en place |
| Parallel WS Messages | Faible | Faible | ⚠️ Limité par RTT |
| Server Colocation | Haut | Très Haut | ❌ V2+ |

---

- [ ] Task 1: Analyze current order flow for micro-optimizations (AC: #1)
  - [ ] 1.1: Profile `execute_delta_neutral()` to identify any remaining internal delays
  - [ ] 1.2: Verify pre-signing is used for Vest (should add ~0.16ms, not blocking)
  - [ ] 1.3: Verify HTTP connection pooling is active (Story 7.2 implementation)
  - [ ] 1.4: Document current flow with timing annotations

- [ ] Task 2: Implement Optimistic Execution pattern (AC: #1, #2)
  - [ ] 2.1: Research "optimistic execution" pattern for delta-neutral trading
  - [ ] 2.2: Consider sending orders without waiting for confirmation (fire-and-forget)
  - [ ] 2.3: Implement async confirmation handling (separate from critical path)
  - [ ] 2.4: Add fallback/rollback logic if confirmation fails

- [ ] Task 3: Investigate parallel order submission improvements (AC: #2)
  - [ ] 3.1: Verify `tokio::join!` is truly parallel (no sequential waits)
  - [ ] 3.2: Profile individual exchange latencies (Vest vs Paradex)
  - [ ] 3.3: Consider staggered submission if one exchange is consistently faster

- [ ] Task 4: Baseline measurement before optimization (AC: #3)
  - [ ] 4.1: Run bot with current code, capture 5+ trades with `SlippageAnalysis` events
  - [ ] 4.2: Document baseline metrics: avg `order_to_confirm`, avg `slippage_bps`
  - [ ] 4.3: Record market conditions (volatility, book depth)

- [ ] Task 5: Implement selected optimizations (AC: #2)
  - [ ] 5.1: Apply chosen optimization from Task 2 or 3 analysis
  - [ ] 5.2: Ensure all tests pass (`cargo test`)
  - [ ] 5.3: Ensure no clippy warnings (`cargo clippy`)

- [ ] Task 6: Post-optimization measurement (AC: #3)
  - [ ] 6.1: Run bot with optimized code, capture 5+ trades
  - [ ] 6.2: Compare metrics vs baseline from Task 4
  - [ ] 6.3: Calculate improvement percentage

- [ ] Task 7: Documentation and completion (AC: #1, #2, #3)
  - [ ] 7.1: Document optimization approach and results
  - [ ] 7.2: Update completion notes with findings
  - [ ] 7.3: If improvement < 10%, document why and propose V2 alternatives

## Dev Notes

### Story 8.1 Key Findings (CRITICAL CONTEXT)

**Live Test Results (2026-02-04):**

| Metric | Value | Interpretation |
|--------|-------|----------------|
| `detection_to_signal_ms` | 0-1ms | ✅ Internal: instant |
| `signal_to_order_ms` | 0ms | ✅ Internal: instant |
| `order_to_confirm_ms` | 385-386ms | 🔴 External: bottleneck |
| Total latency | 385-387ms | Limited by network RTT |

**Conclusion**: 99%+ of latency is **external** (exchange processing + network RTT). Internal bot optimization has essentially zero impact.

### Story 7.1 Implementation Reference

**What's Already Optimized:**
- HTTP connection pooling (Story 7.2): `reqwest::Client` avec keep-alive, warm-up
- Pre-signed orders (Vest): Signature calculated before order submission (~0.16ms)
- Parallel execution: `tokio::join!` for simultaneous Vest + Paradex orders
- No Supabase in critical path (Story 7.3): Database removed from execution flow

### Realistic Optimization Options

> [!CAUTION]
> **ADSL en zone rurale = ~386ms RTT incompressible.** Aucune optimisation côté code ne peut réduire cette latence. Seules des solutions infrastructure peuvent aider.

#### Option A: VPS Cloud (RECOMMANDÉ)
- **Solution**: Déployer le bot sur un VPS (AWS, Hetzner, OVH) proche des serveurs exchange
- **Impact**: Latence potentiellement réduite à 50-100ms
- **Coût**: ~5-20€/mois
- **Effort**: Moyen (setup Docker, deployment)
- **Status**: 🎯 **Seule vraie solution pour réduire le slippage**

#### Option B: Threshold Re-calibration (Palliative)
- **Rationale**: Au lieu d'optimiser la vitesse, ajuster le seuil d'entrée pour compenser le slippage attendu
- **Exemple**: Si slippage moyen = 8bps, augmenter `spread_entry` de 8bps
- **Impact**: Moins de trades mais plus profitables
- **Effort**: Faible (config change)

#### Option C: Accept Current Latency (V1 Status Quo)
- **Rationale**: Documenter la limitation et continuer avec la config actuelle
- **Impact**: Aucun changement, slippage accepté
- **Action**: Fermer cette story avec documentation des findings

#### Option D: Connexion 4G/5G Backup
- **Solution**: Utiliser une connexion mobile comme alternative à l'ADSL
- **Impact**: Variable selon couverture réseau
- **Effort**: Faible (hardware + SIM)

### Architecture Compliance

**Modules to potentially modify:**
- `src/core/execution.rs` - Order execution flow
- `src/core/events.rs` - Metrics logging (if adding new metrics)
- `config.yaml` - If adding configurable thresholds

**Pattern Adherence:**
- Use existing `TradingEvent::slippage_analysis()` for measurements
- Continue using `tracing` macros for structured logging
- Use `thiserror` for any new error variants

### Testing Requirements

- All existing tests must pass: `cargo test`
- No clippy warnings: `cargo clippy -- -D warnings`
- Live validation required with real trades
- Before/after comparison using `SlippageAnalysis` events

### Project Structure Notes

- Story builds directly on Story 8.1's timing infrastructure
- No new modules expected - modifications to existing `execution.rs`
- Slippage metrics already in place from Story 8.1

### References

- [Source: 8-1-slippage-investigation-timing-breakdown.md] - Complete Story 8.1 with findings
- [Source: epics.md#Story-8.2] - Epic definition
- [Source: src/core/events.rs] - `SlippageAnalysis` event and `TimingBreakdown` struct
- [Source: src/core/execution.rs] - Current execution flow with timing captures
- [Source: 7-1-websocket-orders-paradex.md] - Previous latency optimization patterns

### Git Intelligence

**Recent commits (2026-02-04):**
- `625f948` - feat(5.3): implement structured trading event logging
- `f104ec9` - feat: implement exit monitoring in execution_task
- `5489a99` - feat(v1-hft): Remove Supabase + Mutex, reduce polling to 25ms
- `d3e5a44` - feat(7.2): Implement HTTP connection pooling for Vest adapter
- `7a98dc1` - feat(7.1): integrate subscribe_orders() into runtime + add warm_up_http unit test

**Pattern from 7.1**: Baseline measurement → implementation → validation cycle

### Previous Story Intelligence (Story 8.1)

**Key Implementation Details:**
- `TimingBreakdown` struct captures all phase durations
- `TradingEvent::slippage_analysis()` logs comprehensive metrics
- `t_trail` vector in `execute_delta_neutral()` captures timestamps
- Slippage calculation: `(detection_spread - execution_spread) * 100` bps

**The Developer Should Know:**
- Current implementation is already well-optimized for internal processing
- External latency (386ms) is likely incompressible without infrastructure changes
- This story may result in documenting limitations rather than code changes

## Dev Agent Record

### Agent Model Used

{{agent_model_name_version}}

### Debug Log References

### Completion Notes List

### File List
