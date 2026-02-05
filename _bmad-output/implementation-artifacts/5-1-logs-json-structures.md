# Story 5.1: Logs JSON Structurés

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a opérateur,
I want que le bot émette des logs JSON structurés,
so that je puisse les parser et les analyser facilement.

## Acceptance Criteria

1. **Given** le bot en cours d'exécution
   **When** un événement est loggé
   **Then** le log est émis au format JSON
   **And** chaque log contient: timestamp, level, message, fields contextuels
   **And** les logs sont émis sur stdout
   **And** le format est compatible avec des outils comme `jq`

## Tasks / Subtasks

- [x] Task 1: Implement JSON Subscriber Configuration (AC: #1)
  - [x] 1.1 Created `src/config/logging.rs` with `init_logging()` function
  - [x] 1.2 Respects `RUST_LOG` env var via `EnvFilter::try_from_default_env()`
  - [x] 1.3 JSON output includes all structured fields from tracing macros
  - [x] 1.4 Updated `main.rs` to call `config::init_logging()`

- [x] Task 2: Create Logging Configuration (AC: #1)
  - [x] 2.1 Added `LOG_FORMAT` environment variable (values: `json`, `pretty`, default: `json`)
  - [x] 2.2 Implemented conditional subscriber init based on LOG_FORMAT
  - [x] 2.3 Already documented in `.env.example` (line 64)

- [x] Task 3: Update Test Binaries (AC: #1)
  - [x] 3.1 Updated all 7 bin files to use shared `config::init_logging()`
  - [x] 3.2 Files: `monitor.rs`, `test_order.rs`, `test_paradex_order.rs`, `test_auto_close.rs`, `delta_neutral_cycle.rs`, `close_positions.rs`, `get_paradex_address.rs`
  - [x] 3.3 Shared initialization via `hft_bot::config::init_logging()`

- [x] Task 4: Validate JSON Compatibility (AC: #1)
  - [x] 4.1 Build succeeded for all targets
  - [x] 4.2 All 168 tests pass including logging module tests
  - [x] 4.3 Timestamp format is ISO-8601 as per tracing-subscriber JSON layer

## Dev Notes

### Current State Analysis

**Current subscriber in `main.rs` (line 37-39):**
```rust
tracing_subscriber::fmt()
    .with_env_filter("info")
    .init();
```

This outputs **human-readable** format:
```
2026-02-05T02:30:00.123Z  INFO hft_bot::main: Bot runtime started event_type="RUNTIME"
```

### Target State

**JSON format output:**
```json
{"timestamp":"2026-02-05T02:30:00.123456Z","level":"INFO","target":"hft_bot::main","message":"Bot runtime started","event_type":"RUNTIME"}
```

### Implementation Approach

**Recommended implementation:**
```rust
use tracing_subscriber::fmt::format::FmtSpan;

fn init_logging() {
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_string());
    
    if log_format == "pretty" {
        // Human-readable for development
        tracing_subscriber::fmt()
            .with_env_filter("info")
            .pretty()
            .init();
    } else {
        // JSON for production (default)
        tracing_subscriber::fmt()
            .with_env_filter("info")
            .json()
            .init();
    }
}
```

### Dependency Status

**Already in Cargo.toml:**
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
```

> [!NOTE]
> The `json` feature is already included. No new dependencies needed.

### Story 5.3 Integration

**Story 5.3 (in review) created:**
- `src/core/events.rs` — `TradingEvent` struct with `event_type`, `timestamp`, etc.
- Factory methods: `spread_detected()`, `trade_entry()`, `trade_exit()`, etc.
- `log_event()` function that emits structured tracing events

**All Story 5.3 events will automatically render as JSON** once this story enables the JSON subscriber.

### Architecture Compliance

**From architecture.md — Logging Patterns:**
```rust
// Événements business
info!(pair = %pair, spread = spread_pct, "Spread detected");
```

JSON output will preserve all structured fields from `tracing` macros.

### Files to Modify

| File | Changes |
|------|---------|
| `src/main.rs` | Replace `fmt().init()` with `fmt().json().init()` or configurable |
| `src/bin/monitor.rs` | Update subscriber (optional, for consistency) |
| `src/bin/test_order.rs` | Update subscriber (optional) |
| `src/bin/test_paradex_order.rs` | Update subscriber (optional) |
| `src/bin/test_auto_close.rs` | Update subscriber (optional) |
| `src/bin/delta_neutral_cycle.rs` | Update subscriber (optional) |
| `src/bin/close_positions.rs` | Update subscriber (optional) |
| `.env.example` | Document `LOG_FORMAT` variable |

### Testing Standards

- Run `cargo clippy --all-targets -- -D warnings` before commit
- Run `cargo test` to verify no regressions
- Validate JSON output: `RUST_LOG=info cargo run 2>&1 | head -5 | jq .`

### Previous Story Intelligence

**Story 5.3 (Logging des Événements de Trading avec Contexte):**
- Status: `review`
- Created comprehensive `TradingEvent` schema with 8 event types
- Replaced legacy `[TAG]` prefixes with structured `event_type` fields
- All 142 tests pass

**Key learnings from Story 5.3:**
- Event structure already optimized for JSON output
- `timestamp` field uses Unix epoch milliseconds
- No emoji in production logs (already cleaned up)

### Git Recent Commits

```
1c81c4b refactor(adapters): centralize shared patterns in types.rs
5f8b903 refactor(runtime): extract exit_monitoring_loop function
```

### References

- [Source: architecture.md#Logging Patterns] — tracing conventions
- [Source: epics.md#FR19] — Les logs JSON structurés requirement
- [Source: main.rs#L37-39] — Current subscriber configuration
- [Source: 5-3-logging-evenements-trading-contexte.md] — Story 5.3 events infrastructure

## Dev Agent Record

### Agent Model Used

Anthropic Claude Sonnet 4

### Debug Log References

- Build: `cargo build --all-targets` - SUCCESS
- Tests: `cargo test --lib` - 168 passed, 0 failed

### Completion Notes List

1. Created `src/config/logging.rs` with `init_logging()` function
2. Updated `src/config/mod.rs` to export the logging module
3. Updated `main.rs` to use `config::init_logging()`
4. Updated 7 binary files to use shared logging configuration
5. Fixed pre-existing clippy `needless_borrow` error in `paradex/adapter.rs`

### File List

- [NEW] `src/config/logging.rs` - Centralized logging configuration
- [MODIFY] `src/config/mod.rs` - Export logging module
- [MODIFY] `src/main.rs` - Use `config::init_logging()`
- [MODIFY] `src/bin/monitor.rs` - Use shared logging
- [MODIFY] `src/bin/test_order.rs` - Use shared logging
- [MODIFY] `src/bin/test_paradex_order.rs` - Use shared logging
- [MODIFY] `src/bin/test_auto_close.rs` - Use shared logging
- [MODIFY] `src/bin/delta_neutral_cycle.rs` - Use shared logging
- [MODIFY] `src/bin/close_positions.rs` - Use shared logging
- [MODIFY] `src/bin/get_paradex_address.rs` - Use shared logging
- [MODIFY] `src/adapters/paradex/adapter.rs` - Clippy fix (unrelated)

## Senior Developer Review (AI)

**Reviewer:** Antigravity (Claude Sonnet 4)
**Date:** 2026-02-05
**Verdict:** ✅ APPROVED

### Findings Summary
- **2 HIGH** → 1 fixed (H2: placeholder tests), 1 acknowledged (H1: scope mixing - git org only)
- **2 MEDIUM** → Acknowledged (documentation details)
- **2 LOW** → Deferred (style preferences in test binaries)

### H2 Fix Applied
- Replaced placeholder tests in `src/config/logging.rs` with:
  - `test_log_format_default_is_json()` - Validates default logic
  - `test_pretty_format_detection()` - Table-driven test for format detection
  - `test_env_filter_fallback()` - Validates RUST_LOG fallback
- Added documentation explaining unit test limitations for tracing subscriber

### AC Validation
| AC | Status | Evidence |
|----|--------|----------|
| JSON format | ✅ | `logging.rs:39` uses `.json()` |
| Timestamp+level+message | ✅ | tracing-subscriber JSON layer |
| stdout output | ✅ | Default behavior |
| jq compatible | ✅ | Standard JSON |
| LOG_FORMAT configurable | ✅ | `logging.rs:25` |
| All binaries updated | ✅ | 7/7 use `config::init_logging()` |
