# Story 2.4: Retry Logic sur Échec d'Ordre

Status: done

<!-- Note: FR8 implementation. Extends execution.rs with retry wrapper. Key constraint: fixed delay (500ms), max 3 attempts (configurable). Story 2.5 (auto-close) depends on this. -->

## Story

As a **opérateur**,
I want que le bot retente un ordre échoué,
So that les échecs temporaires ne bloquent pas l'exécution.

## Acceptance Criteria

1. **Given** un ordre qui échoue (timeout, rate limit, API error)
   **When** l'échec est détecté
   **Then** l'ordre est retenté jusqu'à 3 fois (configurable via `MAX_RETRY_ATTEMPTS`)
   **And** un délai fixe de 500ms est appliqué entre les retries
   **And** un log `[RETRY] Order failed, attempt 2/3...` est émis à chaque retry
   **And** si tous les retries échouent, un événement d'échec est propagé

## Tasks / Subtasks

- [x] **Task 1**: Ajouter constante `retry_delay_ms()` dans `config/constants.rs` (AC: #1)
  - [x] Subtask 1.1: Définir `retry_delay_ms()` avec default 500ms, env var `RETRY_DELAY_MS`
  - [x] Subtask 1.2: Ajouter à `log_configuration()` pour visibilité
  - [x] Subtask 1.3: Test unitaire pour valeur default et env override

- [x] **Task 2**: Créer fonction générique `retry_order()` dans `execution.rs` (AC: #1)
  - [x] Subtask 2.1: Définir signature: `async fn retry_order<A: ExchangeAdapter>(adapter: &A, order: OrderRequest, exchange_name: &str) -> ExchangeResult<RetryResult>`
  - [x] Subtask 2.2: Implémenter boucle avec `max_retry_attempts()` itérations
  - [x] Subtask 2.3: Appliquer `tokio::time::sleep(Duration::from_millis(retry_delay_ms()))` entre attempts
  - [x] Subtask 2.4: Logger chaque retry avec structured logging: `[RETRY] Order failed, attempt {n}/{max}`

- [x] **Task 3**: Créer `RetryResult` type pour capturer détails (AC: #1)
  - [x] Subtask 3.1: Définir struct `RetryResult { success: bool, attempts: u32, final_error: Option<String>, response: Option<OrderResponse> }`
  - [x] Subtask 3.2: Implémenter `From<RetryResult>` pour `LegStatus` pour intégration avec DeltaNeutralResult

- [x] **Task 4**: Intégrer retry dans `DeltaNeutralExecutor::execute_delta_neutral()` (AC: #1)
  - [x] Subtask 4.1: Remplacer les appels directs `place_order()` par `retry_order()`
  - [x] Subtask 4.2: Préserver l'exécution parallèle avec `tokio::join!` des deux retry_order()
  - [x] Subtask 4.3: Logger le nombre total d'attempts dans le log final

- [x] **Task 5**: Implémenter logging structuré détaillé (AC: #1)
  - [x] Subtask 5.1: Log à chaque retry: `warn!(exchange=%name, attempt=n, max=max, error=?e, "[RETRY] Order failed")`
  - [x] Subtask 5.2: Log en cas d'échec définitif: `error!(exchange=%name, total_attempts=n, "[RETRY] All attempts failed")`
  - [x] Subtask 5.3: Log en cas de succès après retry: `info!(exchange=%name, attempt=n, "[RETRY] Order succeeded after retry")`

- [x] **Task 6**: Tests unitaires (AC: #1)
  - [x] Subtask 6.1: `test_retry_succeeds_first_attempt` - pas de delay si succès immédiat
  - [x] Subtask 6.2: `test_retry_succeeds_second_attempt` - succès après 1 retry
  - [x] Subtask 6.3: `test_retry_all_attempts_fail` - échec après max attempts
  - [x] Subtask 6.4: `test_retry_result_to_leg_status_success` - RetryResult → LegStatus success
  - [x] Subtask 6.5: `test_retry_result_to_leg_status_failed` - RetryResult → LegStatus failed

- [x] **Task 7**: Tests d'intégration (AC: #1)
  - [x] Subtask 7.1: `test_delta_neutral_with_retry_one_leg` - un seul leg retry
  - [x] Subtask 7.2: `test_delta_neutral_with_retry_both_legs` - les deux legs retry

- [x] **Task 8**: Validation finale (AC: #1)
  - [x] Subtask 8.1: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 8.2: `cargo test` tous les tests passent (192 tests)
  - [x] Subtask 8.3: Retry logs verified in tests

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`) - 192 tests passing
- [x] `retry_delay_ms()` ajouté dans constants.rs avec test
- [x] `retry_order()` créé dans execution.rs
- [x] `RetryResult` struct avec From<RetryResult> for LegStatus
- [x] Intégration dans `execute_delta_neutral()` avec tokio::join!
- [x] Logs structurés: `[RETRY] Order failed, attempt N/M`
- [x] Tests unitaires pour retry logic (success, fail)
- [x] Test d'intégration avec delta-neutral executor

## Dev Agent Record

### Agent Model Used

Antigravity (gemini)

### Debug Log References

N/A - All tests passed on first run

### Completion Notes List

- **Task 1**: Added `retry_delay_ms()` to `config/constants.rs` with default 500ms, env var `RETRY_DELAY_MS`. Added to `log_configuration()`. Added test for default value and env override.
- **Task 2-3**: Created `RetryResult` struct capturing success, attempts, final_error, and response. Created `retry_order<A: ExchangeAdapter>()` async function with loop, configurable delay, and structured logging. Implemented `From<RetryResult> for LegStatus`.
- **Task 4**: Replaced direct `place_order()` calls in `execute_delta_neutral()` with `retry_order()`. Preserved parallel execution via `tokio::join!`. Added `long_attempts` and `short_attempts` to trade logs.
- **Task 5**: Implemented all three log levels: `warn!` for each retry, `error!` for all attempts failed, `info!` for success after retry.
- **Task 6-7**: Created `FailNTimesAdapter` mock that fails N times then succeeds. Added 8 new tests covering all retry scenarios.
- **Task 8**: Verified `cargo clippy` clean, all 192 tests pass.

### File List

**Modified:**
- `src/config/constants.rs` - Added `retry_delay_ms()`, updated `log_configuration()`, added tests
- `src/core/execution.rs` - Added `RetryResult`, `retry_order()`, integrated into `execute_delta_neutral()`, added 8 retry tests

> [!NOTE]
> Git diff shows additional changes in `src/adapters/paradex/` and `src/bin/` - these are from Story 2.3, not this story. Retry logic is at execution level per architecture.
