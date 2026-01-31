# Story 0.3: Validation de la Structure Modulaire

Status: review

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a développeur,
I want valider que la structure modulaire (adapters/, core/, config/) est cohérente,
so that l'implémentation du MVP puisse s'appuyer sur une base solide.

## Acceptance Criteria

1. **Given** la structure du projet avec adapters/, core/, config/
   **When** j'inspecte les modules et leurs exports
   **Then** chaque module a un `mod.rs` avec des exports explicites

2. **Given** les imports entre modules
   **When** j'analyse les dépendances
   **Then** les dépendances inter-modules sont unidirectionnelles

3. **Given** le projet entier
   **When** j'exécute `cargo doc`
   **Then** la documentation est générée sans erreurs

## Tasks / Subtasks

- [x] Task 1: Audit des mod.rs (AC: #1)
  - [x] Vérifier `src/adapters/mod.rs` — exports explicites ✅
  - [x] Vérifier `src/core/mod.rs` — contient glob exports `pub use *` → à corriger
  - [x] Vérifier `src/config/mod.rs` — exports explicites ✅
  - [x] Vérifier `src/lib.rs` — exports explicites ✅

- [x] Task 2: Correction des Glob Exports (AC: #1)
  - [x] Remplacer `pub use spread::*;` par exports explicites
  - [x] Remplacer `pub use vwap::*;` par exports explicites
  - [x] Remplacer `pub use state::*;` par exports explicites
  - [x] Ajouter re-exports pour `channels` et `logging` (actuellement absents)

- [x] Task 3: Validation des Dépendances Unidirectionnelles (AC: #2)
  - [x] Documenter le graphe de dépendances
  - [x] Vérifier: config ← core ← adapters (pas de retour)
  - [x] Confirmer: pas de cycles entre modules

- [x] Task 4: Génération Documentation (AC: #3)
  - [x] Exécuter `cargo doc --no-deps`
  - [x] Résoudre les erreurs/warnings de documentation
  - [x] Vérifier les liens inter-modules

- [x] Task 5: Validation Finale
  - [x] `cargo clippy --all-targets -- -D warnings` → 0 warnings
  - [x] `cargo test` → 213+ tests passent

## Dev Notes

### Résumé des changements

**Module Export Refactoring:**
- Refactoret `core/mod.rs` de glob exports (`pub use *`) vers exports explicites
- Ajouté re-exports manquants pour `channels` et `logging` modules
- 27 lignes dans `core/mod.rs` vs 12 avant le refactoring

**Documentation Fixes:**
- `config/types.rs`: Wrapped `Arc<RwLock<AppConfig>>` in backticks (rustdoc HTML tag warning)
- `config/supabase.rs`: Wrapped bare URL in angle brackets (rustdoc link warning)

**Dependency Graph Analysis:**
```
lib.rs
  ├── error ← standalone, no deps
  ├── config ← standalone (env vars, YAML)
  ├── core
  │     ├── spread ← uses adapters::types ✅
  │     ├── vwap ← uses adapters::types ✅
  │     ├── state ← uses config ✅
  │     ├── channels ← standalone ✅
  │     └── logging ← standalone ✅
  └── adapters
        ├── traits ← uses adapters (internal) ✅
        ├── types ← standalone ✅
        └── [vest, paradex] ← use adapters (internal) ✅
```
**Result:** Unidirectional dependencies confirmed. No cycles detected.

### Validation Results

| Check | Result |
|-------|--------|
| `cargo test` | 213 passed, 0 failed |
| `cargo clippy --all-targets -- -D warnings` | 0 warnings |
| `cargo doc --no-deps` | 0 warnings, 0 errors |

### References

- [Source: docs/source-tree.md] — Complete module structure
- [Source: _bmad-output/planning-artifacts/architecture.md#Project-Structure] — Expected boundaries
- [Source: src/adapters/mod.rs] — Good example of explicit exports
- [Source: src/core/mod.rs] — FIXED: now uses explicit exports

## Dev Agent Record

### Agent Model Used

Gemini 2.5 Pro

### Debug Log References

### Completion Notes List

- All 5 tasks completed successfully
- Refactored `core/mod.rs` from glob to explicit exports
- Fixed 2 rustdoc warnings (bare URL, HTML tag)
- All 213 tests pass
- Clippy clean (0 warnings)
- Documentation generates without warnings

### File List

- `src/core/mod.rs` — Refactored from glob to explicit exports
- `src/config/types.rs` — Fixed rustdoc HTML tag warning
- `src/config/supabase.rs` — Fixed rustdoc bare URL warning

### Change Log

| Date | Change |
|------|--------|
| 2026-01-31 | Story 0.3 implementation complete |

