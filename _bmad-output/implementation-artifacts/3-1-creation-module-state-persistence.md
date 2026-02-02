# Story 3.1: Création du Module State Persistence

Status: done

<!-- Note: Ce story pose les fondations de la persistence Supabase pour FR10-12. Le module state.rs existe déjà avec BotState/AppState pour le MVP in-memory, mais ne contient AUCUNE logique Supabase. Ce story ajoute les types Position et StateManager pour la persistence. Stories 3.2-3.4 ajouteront ensuite save/restore/sync. -->

## Story

As a **développeur**,
I want créer le module `src/core/state.rs` avec les structures pour la persistence,
So that la logique de persistence Supabase soit centralisée et prête pour les stories 3.2-3.4.

## Acceptance Criteria

1. **Given** le besoin de persister les positions delta-neutral
   **When** le module `state.rs` est créé/enrichi
   **Then** il exporte les types `PositionState`, `StateManager`
   **And** il est référencé dans `core/mod.rs` (déjà fait — vérifier exports)
   **And** le code compile sans erreurs (`cargo build`)

2. **Given** la structure `PositionState` créée
   **When** j'inspecte ses champs
   **Then** elle contient: `id` (UUID), `pair`, `long_exchange`, `short_exchange`, `long_size`, `short_size`, `entry_spread`, `entry_timestamp`, `status`
   **And** elle dérive `Debug, Clone, Serialize, Deserialize` pour persistence
   **And** elle a une méthode `new()` pour création facile

3. **Given** la structure `StateManager` créée
   **When** j'inspecte son API
   **Then** elle expose des méthodes stub: `save_position()`, `load_positions()`, `update_position()`, `remove_position()`
   **And** chaque méthode retourne un `Result<(), StateError>` ou équivalent
   **And** l'implémentation est un stub qui retourne `Ok(())` (Story 3.2 implémentera la logique Supabase)

## Tasks / Subtasks

- [x] **Task 1**: Analyser l'existant `state.rs` (AC: #1)
  - [x] Subtask 1.1: Lire complètement `src/core/state.rs` — identifier BotState, AppState, Metrics
  - [x] Subtask 1.2: Confirmer que AUCUNE logique Supabase n'existe actuellement
  - [x] Subtask 1.3: Vérifier les exports dans `core/mod.rs` ligne 33

- [x] **Task 2**: Créer le type `PositionState` (AC: #2)
  - [x] Subtask 2.1: Définir `struct PositionState` avec champs: `id: Uuid`, `pair: String`, `long_exchange: String`, `short_exchange: String`
  - [x] Subtask 2.2: Ajouter champs: `long_size: f64`, `short_size: f64`, `entry_spread: f64`, `entry_timestamp: chrono::DateTime<Utc>`
  - [x] Subtask 2.3: Ajouter `status: PositionStatus` enum (Open, Closed, PartialClose)
  - [x] Subtask 2.4: Dériver `Debug, Clone, Serialize, Deserialize`
  - [x] Subtask 2.5: Implémenter méthode `new()` qui génère UUID et timestamp automatiquement

- [x] **Task 3**: Créer le type `StateManager` (AC: #3)
  - [x] Subtask 3.1: Définir `struct StateManager` avec champ `supabase_client: Option<SupabaseClient>` (ou reqwest::Client)
  - [x] Subtask 3.2: Implémenter `new(supabase_url: String, anon_key: String) -> Self`
  - [x] Subtask 3.3: Créer stub `async fn save_position(&self, pos: &PositionState) -> Result<(), StateError>`
  - [x] Subtask 3.4: Créer stub `async fn load_positions(&self) -> Result<Vec<PositionState>, StateError>`
  - [x] Subtask 3.5: Créer stub `async fn update_position(&self, id: Uuid, updates: PositionUpdate) -> Result<(), StateError>`
  - [x] Subtask 3.6: Créer stub `async fn remove_position(&self, id: Uuid) -> Result<(), StateError>`
  - [x] Subtask 3.7: Tous les stubs retournent `Ok(())` pour l'instant

- [x] **Task 4**: Créer le type d'erreur `StateError` (AC: #3)
  - [x] Subtask 4.1: Utiliser `thiserror` pour définir `StateError` enum
  - [x] Subtask 4.2: Ajouter variants: `DatabaseError(String)`, `NotFound`, `InvalidData(String)`, `NetworkError(reqwest::Error)`
  - [x] Subtask 4.3: Dériver `Debug, thiserror::Error`

- [x] **Task 5**: Ajouter exports dans `core/mod.rs` (AC: #1)
  - [x] Subtask 5.1: Ajouter `PositionState, PositionStatus, StateManager, StateError` dans les exports ligne 33
  - [x] Subtask 5.2: Vérifier que `pub use state::{...}` est à jour

- [x] **Task 6**: Ajouter dépendances Cargo.toml (AC: #1)
  - [x] Subtask 6.1: Ajouter `uuid = { version = "1.0", features = ["v4", "serde"] }` si manquant
  - [x] Subtask 6.2: Ajouter `chrono = { version = "0.4", features = ["serde"] }` si manquant
  - [x] Subtask 6.3: Vérifier que `serde`, `serde_json`, `reqwest`, `thiserror` sont présents

- [x] **Task 7**: Tests unitaires de base (AC: #1, #2, #3)
  - [x] Subtask 7.1: `test_position_state_new()` — vérifie génération UUID et timestamp
  - [x] Subtask 7.2: `test_position_state_serialize()` — vérifie serde JSON
  - [x] Subtask 7.3: `test_state_manager_stubs()` — appelle tous les stubs, vérifie Ok(())
  - [x] Subtask 7.4: `test_state_error_variants()` — vérifie tous les variants d'erreur

- [x] **Task 8**: Validation finale (AC: all)
  - [x] Subtask 8.1: `cargo build` compile sans warnings
  - [x] Subtask 8.2: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 8.3: `cargo test` tous les tests passent (~206 tests attendus = 202 + 4 nouveaux)
  - [x] Subtask 8.4: Vérifier exports dans `core/mod.rs`

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`)
- [x] `PositionState` struct créé avec tous les champs requis
- [x] `PositionStatus` enum créé (Open, Closed, PartialClose)
- [x] `StateManager` struct créé avec stubs async
- [x] `StateError` enum créé avec thiserror
- [x] Exports mis à jour dans `core/mod.rs`
- [x] Dependencies ajoutées dans Cargo.toml (uuid, chrono)
- [x] 4+ tests unitaires pour les nouveaux types
- [x] Documentation inline pour les types publics

## Dev Notes

### Architecture Pattern — Natural Key Design (Epic 3 Decision)

**CRITICAL CONTEXT from Epic 2 Retrospective (2026-02-02):**

Le projet a adopté une stratégie **"Natural Key"** SIMPLIFIÉE pour la persistence, basée sur une découverte clé:

**Les exchanges imposent une contrainte naturelle: 1 seule position par (asset, direction).**

Cela simplifie radicalement la reconciliation:
- **Pas besoin de tracking UUID complexe** entre positions in-memory et exchange
- **Reconciliation triviale:** `.find()` par `(exchange, symbol, direction)`
- **Schema simplifié:** UUID PK + UNIQUE(long_symbol, short_symbol) + remaining_size

**Schéma Supabase recommandé:**
```sql
CREATE TABLE positions (
  id UUID PRIMARY KEY,                    -- Generated by PositionState::new()
  long_symbol VARCHAR NOT NULL,           -- e.g., "BTC-PERP"
  short_symbol VARCHAR NOT NULL,          -- e.g., "BTC-USD-PERP"
  long_exchange VARCHAR NOT NULL,         -- "vest"
  short_exchange VARCHAR NOT NULL,        -- "paradex"
  remaining_size NUMERIC NOT NULL,        -- For partial close tracking
  entry_spread NUMERIC NOT NULL,
  entry_timestamp TIMESTAMPTZ NOT NULL,
  status VARCHAR NOT NULL,                -- "open", "closed", "partial"
  created_at TIMESTAMPTZ DEFAULT NOW(),
  updated_at TIMESTAMPTZ DEFAULT NOW(),
  UNIQUE (long_symbol, short_symbol)      -- Natural key constraint
);
```

**Pourquoi `remaining_size` au lieu de `long_size` + `short_size` ?**
- Les exchanges gèrent chaque leg indépendamment
- L'auto-close (Story 2.5) peut fermer une seule leg
- `remaining_size` = quantité encore ouverte des DEUX legs
- Simplifie la logique de partial close

**Impact sur ce Story:**
- `PositionState` DOIT inclure `remaining_size: f64` en PLUS de `long_size/short_size`
- Le champ `status` distingue Open/PartialClose/Closed
- Les champs `long_symbol`/`short_symbol` servent de natural key

### Code Location

- **Fichier à modifier:** `src/core/state.rs` (existe déjà — ~100 lignes)
- **Contenu existant:** BotState, AppState, Metrics — logic in-memory MVP (pas de Supabase)
- **Nouveau code:** PositionState, PositionStatus, StateManager, StateError (~150 lignes)
- **Exports:** `src/core/mod.rs` ligne 33

### Existing Patterns to Follow

**Error Handling (from architecture.md):**
```rust
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Position not found")]
    NotFound,
}
```

**Struct Patterns (from architecture.md):**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionState {
    pub id: Uuid,
    pub pair: String,
    // ... autres champs
}
```

**Async Patterns (from Story 2.5 execution.rs):**
```rust
pub async fn save_position(&self, pos: &PositionState) -> Result<(), StateError> {
    // Stub pour Story 3.1 — implémenté dans Story 3.2
    Ok(())
}
```

### Supabase Config Integration

Le projet a DÉJÀ une configuration Supabase existante (`src/config/supabase.rs`) créée dans une version antérieure (Story 9.7) pour analytics. **RÉUTILISER cette config:**

```rust
// src/config/supabase.rs existe déjà
pub struct SupabaseConfig {
    pub url: String,
    pub anon_key: String,
    pub enabled: bool,
}
```

**StateManager peut instancier le client Supabase avec:**
```rust
impl StateManager {
    pub fn new(config: SupabaseConfig) -> Self {
        let client = if config.enabled {
            Some(reqwest::Client::new()) // Story 3.2 ajoutera la vraie initialisation
        } else {
            None
        };
        Self { supabase_client: client }
    }
}
```

### Testing Requirements

**Baseline:** 202 tests passent actuellement (Epic 2 complete)

**Nouveaux tests attendus (+4 minimum):**
1. `test_position_state_new()` — Génération UUID/timestamp
2. `test_position_state_serialize()` — Serde roundtrip
3. `test_state_manager_stubs()` — Appels stub OK
4. `test_state_error_variants()` — Error display

### Dependencies to Add (if missing)

```toml
[dependencies]
uuid = { version = "1.0", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
reqwest = { version = "0.11", features = ["json"] }
```

### References

- [Source: epics.md#Epic-3 Story 3.1] Requirement de base
- [Source: architecture.md#Files-to-Create] `src/core/state.rs` priorité High
- [Source: architecture.md#Data-Architecture] Schema minimal extensible
- [Source: sprint-status.yaml#L109-111] Natural Key design decision
- [Source: 2-5-auto-close-echec-leg.md] Pattern reduce_only pour partial close
- [Source: src/config/supabase.rs] Config existante à réutiliser

## Dev Agent Record

### Agent Model Used

Claude 3.7 Sonnet (Antigravity)

### Completion Notes

**Story 3.1: State Persistence Foundation - COMPLETE ✅**

**Implementation Summary:**
Successfully created the foundational state persistence module for Epic 3 (FR10-12). Added 5 new types to `src/core/state.rs` (~210 lines) implementing the Natural Key design from Epic 2 retrospective.

**Types Created:**
1. **`PositionState`**: Delta-neutral position with Natural Key fields (`long_symbol`, `short_symbol`) + `remaining_size` for partial close support
2. **`PositionStatus`**: Enum (Open, PartialClose, Closed)
3. **`StateManager`**: Supabase persistence manager with 4 stub async methods (save, load, update, remove)
4. **`StateError`**: thiserror-based error enum (DatabaseError, NotFound, InvalidData, NetworkError)
5. **`PositionUpdate`**: Partial update struct for position modifications

**Key Design Decisions:**
- **Natural Key Approach**: Leverages exchange constraint (1 position per asset/direction) for simplified reconciliation
- **`remaining_size` Field**: Supports partial close (Story 2.5 auto-close compatibility)
- **Stub Implementation**: All StateManager methods return Ok(()) — Stories 3.2-3.4 will add actual Supabase logic

**Testing:**
- Added 4 comprehensive unit tests (+124 lines)
- Test baseline: 202 → 206 tests (100% pass rate)
- Coverage: UUID generation, serialization, stub methods, error display

**Validation Results:**
- ✅ `cargo build`: Clean compile (1m 39s)
- ✅ `cargo clippy --all-targets -- -D warnings`: Zero warnings
- ✅ `cargo test --lib`: 206/206 tests passed

**Dependencies Configured:**
- Updated `uuid` with `serde` feature (Cargo.toml line 44)
- All required dependencies present (chrono, thiserror, reqwest, serde)

**Exports Updated:**
- `core/mod.rs` now exports all Epic 3 types with inline comment annotation

**Next Steps:**
- Story 3.2: Implement actual Supabase save logic in StateManager
- Story 3.3: Implement position restoration after restart
- Story 3.4: In-memory state coherence

### File List

**Modified:**
- `src/core/state.rs` (+210 lines: PositionState, StateManager, tests)
- `src/core/mod.rs` (updated exports for Epic 3 types)
- `Cargo.toml` (added serde feature to uuid)

**No files created or deleted**
