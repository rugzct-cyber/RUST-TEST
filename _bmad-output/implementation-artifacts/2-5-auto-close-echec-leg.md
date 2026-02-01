# Story 2.5: Auto-Close sur Échec de Leg

Status: review

<!-- Note: NFR7 implementation. Extends execute_delta_neutral in execution.rs. When one leg fails after retry, auto-close the successful leg using reduce_only=true. Story 2.4 (retry) must be complete. -->

## Story

As a **opérateur**,
I want que le bot ferme automatiquement la leg réussie si l'autre échoue,
So that je n'aie jamais de position directionnelle non couverte (NFR7).

## Acceptance Criteria

1. **Given** une exécution delta-neutral où une leg a réussi et l'autre a échoué après tous les retries
   **When** l'échec définitif est confirmé
   **Then** la leg réussie est automatiquement fermée (ordre opposé avec `reduce_only=true`)
   **And** un log `[SAFETY] Auto-closing successful leg to avoid exposure` est émis
   **And** aucune position directionnelle non couverte ne reste ouverte
   **And** l'état final est loggé avec le résumé de l'opération

2. **Given** les deux legs échouent après tous les retries
   **When** l'échec définitif est confirmé
   **Then** aucune action de close n'est nécessaire (pas de position ouverte)
   **And** un log `[TRADE] Both legs failed - no position to close` est émis

3. **Given** les deux legs réussissent
   **When** l'exécution complète
   **Then** aucune action de close n'est déclenchée (comportement normal)

## Tasks / Subtasks

- [x] **Task 1**: Créer helper `create_closing_order()` dans `execution.rs` (AC: #1)
  - [x] Subtask 1.1: Définir fonction qui prend `OrderResponse` et retourne `OrderRequest` avec `reduce_only=true`
  - [x] Subtask 1.2: La side doit être inversée (Buy → Sell, Sell → Buy)
  - [x] Subtask 1.3: Utiliser même symbole et quantité que l'ordre original
  - [x] Subtask 1.4: Utiliser `OrderType::Market` pour fermeture immédiate

- [x] **Task 2**: Implémenter async fn `auto_close_leg()` (AC: #1)
  - [x] Subtask 2.1: Prend adapter, successful `OrderResponse`, exchange_name
  - [x] Subtask 2.2: Appelle `create_closing_order()` puis `retry_order()` (réutilise retry logic de Story 2.4)
  - [x] Subtask 2.3: Logger `[SAFETY] Auto-closing successful leg to avoid exposure, exchange=%s`
  - [x] Subtask 2.4: Retourne `AutoCloseResult { success: bool, attempts: u32, error: Option<String> }`

- [x] **Task 3**: Créer `AutoCloseResult` type (AC: #1)
  - [x] Subtask 3.1: Définir struct avec `success`, `attempts`, `close_response: Option<OrderResponse>`, `error: Option<String>`
  - [x] Subtask 3.2: Ajouter méthode `was_needed() -> bool` pour distinguer "no close needed" vs "close failed"

- [x] **Task 4**: Modifier `execute_delta_neutral()` pour auto-close (AC: #1, #2, #3)
  - [x] Subtask 4.1: Après check `success = long_status.is_success() && short_status.is_success()`
  - [x] Subtask 4.2: Si `!success` et une seule leg a réussi: appeler `auto_close_leg()` sur la leg réussie
  - [x] Subtask 4.3: Si les deux legs ont échoué: logger `[TRADE] Both legs failed - no position to close`
  - [x] Subtask 4.4: Ajouter `auto_close_result: Option<AutoCloseResult>` dans `DeltaNeutralResult`
  - [x] Subtask 4.5: Logger l'état final avec résumé de l'opération

- [x] **Task 5**: Logging structuré détaillé (AC: #1, #2)
  - [x] Subtask 5.1: Log pré-close: `warn!(exchange=%name, "[SAFETY] Initiating auto-close for exposed leg")`
  - [x] Subtask 5.2: Log succès: `info!(exchange=%name, attempts=n, "[SAFETY] Successfully closed exposed leg")`
  - [x] Subtask 5.3: Log échec: `error!(exchange=%name, attempts=n, "[SAFETY] CRITICAL: Failed to close exposed leg")`
  - [x] Subtask 5.4: Log both-failed: `info!("[TRADE] Both legs failed - no position to close")`

- [x] **Task 6**: Tests unitaires (AC: #1, #2, #3)
  - [x] Subtask 6.1: `test_auto_close_triggered_when_one_leg_fails` - vérifie qu'auto-close est appelé
  - [x] Subtask 6.2: `test_auto_close_not_triggered_when_both_succeed` - vérifie pas de close si succès
  - [x] Subtask 6.3: `test_auto_close_not_triggered_when_both_fail` - vérifie pas de close si deux échecs
  - [x] Subtask 6.4: `test_create_closing_order_inverts_side` - vérifie inversion Buy↔Sell
  - [x] Subtask 6.5: `test_create_closing_order_sets_reduce_only` - vérifie reduce_only=true
  - [x] Subtask 6.6: `test_auto_close_result_to_delta_neutral_result` - vérifie intégration

- [x] **Task 7**: Tests d'intégration (AC: #1, #2)
  - [x] Subtask 7.1: `test_delta_neutral_with_auto_close_long_fails` - long échoue, short fermé
  - [x] Subtask 7.2: `test_delta_neutral_with_auto_close_short_fails` - short échoue, long fermé

- [x] **Task 8**: Validation finale (AC: all)
  - [x] Subtask 8.1: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 8.2: `cargo test` tous les tests passent (~200 tests attendus)
  - [x] Subtask 8.3: Vérifier les logs [SAFETY] dans les tests

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`)
- [x] `auto_close_leg()` créé dans execution.rs
- [x] `AutoCloseResult` struct créé
- [x] `execute_delta_neutral()` modifié pour auto-close
- [x] Logs structurés: `[SAFETY] Auto-closing successful leg to avoid exposure`
- [x] Tests unitaires pour auto-close logic (8 tests)
- [x] Tests d'intégration avec delta-neutral executor (2 tests)
- [x] NFR7 compliance vérifié (no single-leg exposure)

## Dev Notes

### Architecture Pattern

Le flow d'auto-close s'intègre dans `execute_delta_neutral()` après l'évaluation du succès des deux legs:

```
execute_delta_neutral()
├── tokio::join!(retry_order(vest), retry_order(paradex))
├── Évaluer success = long.is_success() && short.is_success()
├── if !success && exactly_one_succeeded:
│   └── auto_close_leg(successful_adapter, successful_response)
└── Return DeltaNeutralResult with auto_close_result
```

### Code Location

- **Fichier principal:** `src/core/execution.rs`
- **Types ajoutés:** `AutoCloseResult`, `create_closing_order()`
- **Fonction créée:** `auto_close_leg()`
- **Fonction modifiée:** `execute_delta_neutral()`

### Pattern `reduce_only`

Les adapters Vest et Paradex supportent déjà `reduce_only=true`:
- **Vest:** Testé dans Story 2.1 (round-trip BUY + SELL with reduce_only)
- **Paradex:** Testé dans Story 2.2/2.3 (position lifecycle with reduce_only)

### References

- [Source: epics.md#Epic-2 Story 2.5] FR9 requirement
- [Source: architecture.md#Resilience-Patterns] NFR7: Auto-close opposite leg
- [Source: execution.rs#L295-L318] Current partial failure handling (no close)
- [Source: 2-4-retry-logic-echec-ordre.md] Retry logic pattern to reuse

## Dev Agent Record

### Agent Model Used

Gemini 2.5 (Antigravity)

### Completion Notes List

- Implemented AutoCloseResult struct with three constructors: `not_needed()`, `closed()`, `failed()`
- Implemented `create_closing_order()` helper that inverts side and sets reduce_only=true
- Implemented `auto_close_leg()` async function that uses retry_order for resilience
- Modified `execute_delta_neutral()` with match pattern for (long_success, short_success)
- Added 8 comprehensive unit tests covering all acceptance criteria
- All 200 tests pass, clippy clean

### File List

- `src/core/execution.rs` (modified)
