# Story 1.5: Détection de Dépassement de Seuil

Status: review

<!-- Note: Epic 1 Story 5 - Add threshold filtering to SpreadMonitor to detect when spreads exceed configured thresholds. -->

## Story

As a **opérateur**,
I want que le bot détecte quand un spread dépasse le seuil configuré,
So that je sois alerté des opportunités de trade.

## Acceptance Criteria

1. **Given** un seuil de spread configuré dans `config.yaml` (ex: 0.30%)
   **When** le spread calculé dépasse ce seuil
   **Then** un log `[INFO] Spread opportunity detected: spread=X%, threshold=Y%` est émis
   **And** un événement `SpreadOpportunity` est envoyé sur le channel d'exécution
   **And** le seuil peut être entry ou exit selon configuration

## Tasks / Subtasks

- [x] **Task 1**: Ajouter les seuils au SpreadMonitor (AC: #1)
  - [x] Subtask 1.1: Ajouter champs `entry_threshold: f64` et `exit_threshold: f64` à `SpreadMonitor`
  - [x] Subtask 1.2: Modifier `SpreadMonitor::new()` pour accepter les seuils en paramètres
  - [x] Subtask 1.3: Créer struct `SpreadThresholds { entry: f64, exit: f64 }` pour regrouper

- [x] **Task 2**: Modifier la logique de détection (AC: #1)
  - [x] Subtask 2.1: Dans `handle_orderbook_update()`, remplacer `entry_spread > 0.0` par `entry_spread >= self.entry_threshold`
  - [x] Subtask 2.2: Remplacer `exit_spread > 0.0` par `exit_spread >= self.exit_threshold`
  - [x] Subtask 2.3: Inclure le threshold dans le log structuré: `info!(spread=%, threshold=%)`

- [x] **Task 3**: Mettre à jour les logs structurés (AC: #1)
  - [x] Subtask 3.1: Format log: `[INFO] Spread opportunity detected: spread=X%, threshold=Y%`
  - [x] Subtask 3.2: Ajouter champ `threshold` au log avec `tracing` macros
  - [x] Subtask 3.3: Distinguer entry vs exit threshold dans le message

- [x] **Task 4**: Tests unitaires (AC: #1)
  - [x] Subtask 4.1: `test_spread_monitor_detects_entry_above_threshold`
  - [x] Subtask 4.2: `test_spread_monitor_ignores_entry_below_threshold`
  - [x] Subtask 4.3: `test_spread_monitor_detects_exit_above_threshold`
  - [x] Subtask 4.4: `test_spread_monitor_ignores_exit_below_threshold`
  - [x] Subtask 4.5: Test avec seuils différents pour entry et exit

- [x] **Task 5**: Validation finale (AC: #1)
  - [x] Subtask 5.1: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 5.2: `cargo test` tous les tests passent (237 tests)
  - [x] Subtask 5.3: Vérifier logs avec seuils configurés

## Dev Notes

### Contexte Brownfield — Code Existant

> ⚠️ **CRITICAL**: Ce projet est brownfield avec ~8,900 lignes existantes. Le SpreadMonitor existe déjà et émet des opportunités. Cette story AJOUTE la comparaison de seuils.

**L'objectif est de MODIFIER le SpreadMonitor existant pour filtrer les spreads par rapport aux seuils configurés.**

### Analyse du Code Existant

| Composant | Status | Fichier | Lignes |
|-----------|--------|---------|--------|
| `SpreadMonitor` | ✅ Existe | `core/spread.rs` | 276-395 |
| `SpreadMonitor.run()` | ✅ Existe | `core/spread.rs` | 298-316 |
| `SpreadMonitor.handle_orderbook_update()` | ✅ Existe | `core/spread.rs` | 319-388 |
| `SpreadOpportunity` | ✅ Existe | `core/channels.rs` | 17-25 |
| `BotConfig.spread_entry` | ✅ Existe | `config/types.rs` | via loader |
| `BotConfig.spread_exit` | ✅ Existe | `config/types.rs` | via loader |
| Threshold filtering | ❌ À ajouter | `core/spread.rs` | — |

### Architecture Guardrails

**Fichiers à modifier :**
- `src/core/spread.rs` — ajouter les seuils au `SpreadMonitor` et modifier la logique de détection

**Fichiers à NE PAS modifier :**
- `src/core/channels.rs` — channels déjà configurés
- `src/config/loader.rs` — chargement config déjà fonctionnel
- `src/adapters/` — pas de changements

**Patterns obligatoires (copiés de Story 1.4) :**

```rust
// SpreadMonitor structure MISE À JOUR
pub struct SpreadMonitor {
    calculator: SpreadCalculator,
    vest_orderbook: Option<Orderbook>,
    paradex_orderbook: Option<Orderbook>,
    pair: String,
    entry_threshold: f64,  // [NEW] Story 1.5
    exit_threshold: f64,   // [NEW] Story 1.5
}

// Constructor updated
impl SpreadMonitor {
    pub fn new(pair: impl Into<String>, entry_threshold: f64, exit_threshold: f64) -> Self {
        Self {
            calculator: SpreadCalculator::new("vest", "paradex"),
            vest_orderbook: None,
            paradex_orderbook: None,
            pair: pair.into(),
            entry_threshold,
            exit_threshold,
        }
    }
}

// Detection logic MISE À JOUR dans handle_orderbook_update()
if entry_spread >= self.entry_threshold {
    info!(
        pair = %self.pair,
        spread = %format!("{:.4}%", entry_spread),
        threshold = %format!("{:.4}%", self.entry_threshold),
        direction = "entry",
        "Spread opportunity detected"
    );
    // ... emit SpreadOpportunity ...
}

if exit_spread >= self.exit_threshold {
    info!(
        pair = %self.pair,
        spread = %format!("{:.4}%", exit_spread),
        threshold = %format!("{:.4}%", self.exit_threshold),
        direction = "exit",
        "Spread opportunity detected"
    );
    // ... emit SpreadOpportunity ...
}
```

### Sprint Status Current Location

Le code existant dans `handle_orderbook_update()` (lines 344-384) fait actuellement:
```rust
// BEFORE (Story 1.4):
if entry_spread > 0.0 {  // Émet si spread positif
    // ...
}
```

Cette story change vers:
```rust
// AFTER (Story 1.5):
if entry_spread >= self.entry_threshold {  // Émet si spread >= seuil configuré
    // ...
}
```

### Configuration Context

**Config YAML existante (déjà définie):**
```yaml
bots:
  - id: btc_vest_paradex
    pair: BTC-PERP
    spread_entry: 0.30    # Entry threshold = 0.30%
    spread_exit: 0.05     # Exit threshold = 0.05%
    # ...
```

**Les seuils sont DÉJÀ chargés** via `BotConfig` dans `config/types.rs`. Il faut les passer au `SpreadMonitor` lors de l'initialisation.

### Project Structure Notes

**Structure après cette story :**
```
src/core/
├── mod.rs           # Exports (pas de changement)
├── spread.rs        # SpreadMonitor avec seuils (MODIFIÉ)
├── channels.rs      # SpreadOpportunity (pas de changement)
├── vwap.rs          # VWAP calculator
└── state.rs         # Supabase persistence
```

### Technical Requirements

**Threshold Validation:**
- Les seuils sont en pourcentage (0.30 = 0.30%)
- Doivent être > 0 et < 100
- Entry threshold typiquement plus élevé que exit (0.30% vs 0.05%)

**Performance :**
- Pas d'impact sur NFR1 (<2ms) — comparaison f64 triviale
- Pas d'allocations ajoutées dans le hot path

### Previous Story Intelligence

**Story 1.4 (Calcul de Spread Entry/Exit) — TERMINÉE :**
- `SpreadMonitor` créé avec `run()` async loop
- `handle_orderbook_update()` calcule spreads et émet opportunités
- Actuellement émet si `entry_spread > 0.0` (PAS de seuil)
- Logs structurés avec `tracing` macros

**Code Review Story 1.4 Fixes Applied:**
- Dual spread emission (entry AND exit)
- Direction correcte pour exit (BOverA)
- Champ `direction=entry/exit` dans logs

**Leçons apprises des stories précédentes :**
1. Utiliser `tracing` macros avec structured fields (pair, spread, threshold, direction)
2. Modifier les tests existants SI le constructor change
3. Pattern TDD: écrire tests failing, puis implémenter

### ⚠️ Points d'Attention

1. **Constructor Breaking Change**: `SpreadMonitor::new()` aura de nouveaux paramètres
   - Mettre à jour tous les call sites (tests!)
   - Option: créer `SpreadMonitor::new_with_thresholds()` pour backward compat

2. **Existing Tests**: 5 tests SpreadMonitor existants à mettre à jour
   - `test_spread_monitor_processes_orderbook_update`
   - `test_spread_opportunity_emitted_on_spread_change`
   - `test_spread_monitor_handles_missing_orderbook`
   - etc.

3. **Threshold Units**: Les seuils sont en % (0.30 = 0.30%, pas 30%)

### Git Intelligence Summary

**Last 5 commits:**
- `77b70ec`: fix(story-1.4): code review fixes - dual spread emission
- `de02bcf`: feat(story-1.4): Implement SpreadMonitor
- `940a1d1`: fix(story-1.3): Code review fixes - Vest sorting
- `4df19ac`: feat(story-1.3): Implement orderbook parsing
- `01aa0f5`: Story 1.2: Add WebSocket integration tests

**Patterns from recent commits:**
- Préfixe: `feat(story-X.Y):` ou `fix(story-X.Y):`
- Clean commit avec description multi-ligne
- Clippy propre avant push

### References

- [Source: architecture.md#Performance] — NFR2 < 500ms pour detection-to-order (inclut cette étape)
- [Source: epics.md#Story 1.5] — Acceptance criteria originaux
- [Source: spread.rs#SpreadMonitor] — Struct existante (L276-281)
- [Source: spread.rs#handle_orderbook_update] — Logique actuelle (L319-388)
- [Source: config/loader.rs#test] — Config YAML avec spread_entry/spread_exit
- [Source: Story 1.4] — Implémentation précédente (SpreadMonitor créé)

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`) - 237 tests OK
- [x] `SpreadMonitor` accepte `entry_threshold` et `exit_threshold`
- [x] Détection conditionnelle: émet SEULEMENT si `spread >= threshold`
- [x] Log avec le format: `spread=X%, threshold=Y%`
- [x] Tests: 8 nouveaux tests pour threshold filtering
- [x] Tests existants mis à jour pour nouveau constructor

## Dev Agent Record

### Agent Model Used

Claude Sonnet 4

### Change Log

| Date | Change | Files |
|------|--------|-------|
| 2026-01-31 | Added SpreadThresholds struct and threshold filtering logic | src/core/spread.rs |
| 2026-01-31 | Updated existing tests + added 8 new threshold tests | src/core/spread.rs |

### Completion Notes List

- Created `SpreadThresholds` struct with `new()` and `Default` implementations
- Modified `SpreadMonitor` struct to include `entry_threshold` and `exit_threshold` fields
- Updated `SpreadMonitor::new()` to accept threshold parameters
- Added `SpreadMonitor::with_thresholds()` factory method
- Changed detection logic: `entry_spread >= self.entry_threshold` and `exit_spread >= self.exit_threshold`
- Enhanced structured logs with `threshold` field in tracing macros
- Updated 4 existing SpreadMonitor tests for new constructor signature
- Added 8 new threshold-specific tests (3 unit, 5 async)
- All 237 tests pass, clippy clean

### File List

| File | Action | Description |
|------|--------|-------------|
| src/core/spread.rs | MODIFIED | Added SpreadThresholds struct, threshold fields to SpreadMonitor, threshold detection logic, threshold in logs, updated and new tests |
