# Story 0.2: Suppression du Code v3 Résiduel

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a développeur,
I want nettoyer le code résiduel et réparer les erreurs de linter identifiées,
so that le codebase soit propre, maintenable et sans warnings avant l'implémentation du MVP.

## Acceptance Criteria

1. **Given** la fonction `sign_order_message` dans `adapters/paradex.rs` avec trop d'arguments
   **When** je refactorise les arguments dans une struct `OrderSignParams`
   **Then** l'erreur `clippy::too_many_arguments` disparaît

2. **Given** une logique redondante dans `core/logging.rs` (`sanitize_signature`)
   **When** je simplifie les conditions
   **Then** l'erreur `clippy::if_same_then_else` disparaît

3. **Given** des implémentations manuelles de `Default` dans `config/types.rs`
   **When** j'utilise `#[derive(Default)]` ou simplifie le code
   **Then** l'erreur `clippy::derivable_impls` disparaît

4. **Given** le projet entier
   **When** j'exécute `cargo clippy --all-targets -- -D warnings`
   **Then** la compilation passe sans aucun warning

## Tasks / Subtasks

- [x] Task 1: Refactorisation Paradex Adapter
  - [x] Créer struct `OrderSignParams` dans `src/adapters/paradex.rs`
  - [x] Mettre à jour `sign_order_message` pour utiliser cette struct
  - [x] Mettre à jour les appels à cette fonction (si existants)

- [x] Task 2: Correction Logging Logic
  - [x] Simplifier `sanitize_signature` dans `src/core/logging.rs` (fusionner les branches else/if)

- [x] Task 3: Optimisation Config Types
  - [x] Simplifier `impl Default` pour `RiskConfig`, `ApiConfig`, `AppConfig` dans `src/config/types.rs`

- [x] Task 4: Audit Concurrency (Mutex/RwLock)
  - [x] Vérifier que l'usage de `Mutex` pour `ws_stream`/`ws_sender` dans les adapters est optimal
  - [x] Confirmer que `RwLock` est bien utilisé pour `SharedOrderbooks` (préférence lecture)
  - [x] Corriger le commentaire dans `traits.rs` qui parle de "arc-mutex" (devrait être "arc-rwlock")

- [x] Task 5: Validation Finale
  - [x] Exécuter `cargo clippy` (doit être clean)
  - [x] Exécuter `cargo test` (tout doit passer)

## Dev Notes

### Détails Techniques

   ```rust
   pub struct OrderSignParams<'a> {
       pub private_key: &'a str,
       pub account_address: &'a str,
       pub market: &'a str,
       // ... autres champs
   }
   ```
   Cela améliore la lisibilité et la maintenabilité.

2. **Logging Logic**:
   La fonction `sanitize_signature` a deux branches qui retournent "REDACTED". Elles doivent être fusionnées.

3. **Config Types**:
   Vérifier si `#[derive(Default)]` suffit ou si les valeurs par défaut spécifiques (ex: port 8080) nécessitent une implémentation manuelle mais simplifiée.

### References

- `src/adapters/paradex.rs`: ligne ~678
- `src/core/logging.rs`: ligne ~143
- `src/config/types.rs`: ligne ~185

## Dev Agent Record

### File List

| File | Change Type | Description |
|------|-------------|-------------|
| `src/adapters/paradex.rs` | MODIFIED | Added `OrderSignParams` struct (L654-667), refactored `sign_order_message` to use params struct |
| `src/core/logging.rs` | MODIFIED | Simplified `sanitize_signature` function (L140-146) |
| `src/config/types.rs` | MODIFIED | Optimized `Default` impls for `RiskConfig`, `ApiConfig` (L185-201) |
| `src/adapters/traits.rs` | MODIFIED | Fixed concurrency comment at L121 ("arc-mutex" → "arc-rwlock") |

### Change Log

| Date | Task | Changes |
|------|------|---------|
| 2026-01-31 | Task 1 | Created `OrderSignParams<'a>` struct with 10 fields, refactored `sign_order_message` signature |
| 2026-01-31 | Task 2 | Simplified `sanitize_signature` to single clean branch logic |
| 2026-01-31 | Task 3 | Kept manual `impl Default` for `RiskConfig`/`ApiConfig` with specific non-zero values |
| 2026-01-31 | Task 4 | Verified Mutex/RwLock usage, fixed comment in `traits.rs:121` |
| 2026-01-31 | Task 5 | Validated: clippy 0 warnings, 213 tests pass |

### Review Notes

- **Code Review Date:** 2026-01-31
- **Reviewer:** AI Code Review Agent
- **Result:** ✅ All ACs verified, all tasks complete
