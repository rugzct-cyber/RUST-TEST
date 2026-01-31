# Story 0.1: Suppression du Pattern Scout

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a développeur,
I want supprimer le pattern "scout" du codebase,
so that l'architecture soit simplifiée et alignée avec le MVP.

## Acceptance Criteria

1. **Given** le codebase actuel contient des références au pattern "scout"
   **When** je recherche `scout` dans tout le projet
   **Then** aucune référence au pattern scout n'existe ✅

2. **And** le code compile sans erreurs (`cargo build`) ✅

3. **And** tous les tests existants passent (`cargo test`) ✅ (213 passed)

## Tasks / Subtasks

- [x] Task 1: Identifier toutes les références au pattern scout (AC: #1)
  - [x] Rechercher `scout` dans tous les fichiers avec `grep -r "scout" src/`
  - [x] Lister les fichiers et lignes concernés
  - [x] Documenter les fonctions/structs/modules à supprimer

- [x] Task 2: Supprimer le code scout (AC: #1)
  - [x] Supprimer les fonctions/méthodes contenant `scout`
  - [x] Supprimer les structs/enums liés au scout pattern
  - [x] Supprimer les imports inutilisés après suppression
  - [x] Nettoyer les modules `mod.rs` si nécessaires

- [x] Task 3: Valider la compilation (AC: #2)
  - [x] Exécuter `cargo build` et corriger les erreurs de compilation
  - [x] Exécuter `cargo clippy --all-targets -- -D warnings`

- [x] Task 4: Vérifier que les tests passent (AC: #3)
  - [x] Exécuter `cargo test` et s'assurer que tous les tests passent
  - [x] Si des tests sont liés au scout, les supprimer également

## Dev Notes

### Contexte Architectural

Ce cleanup fait partie de la **Phase 0: Cleanup & Foundation** du projet bot4 (HFT Arbitrage Bot). L'objectif est de nettoyer le code legacy v3 avant l'implémentation du MVP.

**Rationale:**
- Pattern "scout" inexistant dans la définition du MVP
- Simplifie l'architecture vers une exécution directe
- Réduit la dette technique avant nouvelles fonctionnalités

### Structure du Projet à Analyser

```
src/
├── main.rs             # Entry point - vérifier si scout référencé
├── lib.rs              # Module exports
├── adapters/           # Exchange adapters (~5,000 lines)
│   ├── vest.rs         # 2140 lines - chercher scout
│   └── paradex.rs      # 2868 lines - chercher scout
├── core/               # Business logic (~2,500 lines)
│   ├── spread.rs       # 737 lines - SpreadCalculator
│   ├── vwap.rs         # 480 lines - VWAP
│   ├── state.rs        # 152 lines - SharedAppState
│   └── channels.rs     # 83 lines - ChannelBundle
└── config/             # Configuration (~500 lines)
```

### Patterns Rust Obligatoires

- **Error Handling:** Utiliser `thiserror` pour erreurs custom, `?` pour propagation
- **Logging:** Utiliser les macros `tracing` (info!, error!, debug!)
- **Conventions:** `snake_case` fonctions/modules, `PascalCase` structs/enums

### Commandes de Validation

```bash
# Vérifier l'absence de scout après cleanup
grep -r "scout" src/

# Build validation
cargo build

# Clippy validation (required before commit)
cargo clippy --all-targets -- -D warnings

# Test validation
cargo test
```

### Project Structure Notes

- Le projet suit une structure modulaire: `adapters/`, `core/`, `config/`
- Brownfield codebase avec ~8,900 lignes existantes
- Dépendances inter-modules doivent rester unidirectionnelles

### References

- [Source: `_bmad-output/planning-artifacts/architecture.md#Cleanup Required (Phase 0)`]
- [Source: `_bmad-output/planning-artifacts/epics.md#Story 0.1: Suppression du Pattern Scout`]
- [Source: `docs/source-tree.md#Source Files by Module`]

## Dev Agent Record

### Agent Model Used

Gemini 2.5 Pro (Antigravity)

### Debug Log References

- `grep_search "scout" src/` → Found 2 references (comments only)
- `cargo build` → Success (dev profile)
- `cargo clippy --all-targets -- -D warnings` → 5 pre-existing errors in unmodified files
- `cargo test` → 213 passed; 0 failed

### Completion Notes List

1. ✅ Identified 2 scout references in codebase (both comments only):
   - `src/core/channels.rs:33` - `/// Scout -> Main: spread opportunities`
   - `src/core/spread.rs:9` - `//! - SpreadTick: Event struct for mpsc broadcast to Scout tasks`

2. ✅ Removed both scout references by updating comments:
   - `channels.rs:33` → `/// SpreadCalculator -> Executor: spread opportunities`
   - `spread.rs:9` → `//! - SpreadTick: Event struct for mpsc broadcast to execution tasks`

3. ✅ Build validation passed (`cargo build` success)

4. ⚠️ Pre-existing clippy errors (5 errors in files NOT modified by this story):
   - `paradex.rs:678` - too_many_arguments
   - `config/types.rs:204` - derivable_impls
   - `core/logging.rs:143` - if_same_then_else
   - These should be addressed in Story 0.2 (Suppression Code v3 Résiduel)

5. ✅ All 213 tests pass (`cargo test` success)

### File List

- `src/core/channels.rs` (modified) - Updated comment line 33
- `src/core/spread.rs` (modified) - Updated comment line 9

