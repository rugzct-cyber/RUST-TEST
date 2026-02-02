# Contributing to bot4

## Development Guidelines

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture
```

### Test Isolation with `#[serial(env)]`

**When to use:** Tests that modify environment variables must use `#[serial(env)]` to prevent race conditions.

**Why:** Environment variables are process-global. When multiple tests run in parallel and modify env vars (e.g., setting API keys, changing config), they can interfere with each other causing flaky test failures.

**Pattern:**

```rust
use serial_test::serial;

#[tokio::test]
#[serial(env)]  // ← Ensures this test runs alone when modifying env vars
async fn test_config_from_env() {
    std::env::set_var("API_KEY", "test_key");
    // ... test code that depends on env var
    std::env::remove_var("API_KEY");
}
```

**Common scenarios requiring `#[serial(env)]`:**
- Tests calling `Config::from_env()`
- Tests setting/modifying `VEST_*`, `PARADEX_*` env vars
- Tests changing `RUST_LOG` or other process-wide config
- Integration tests that initialize adapters with env-based config

**Dependency:** Add to `Cargo.toml`:
```toml
[dev-dependencies]
serial_test = "3.0"
```

---

## Code Style

- Follow `rustfmt` formatting: `cargo fmt`
- Pass `clippy` without warnings: `cargo clippy --all-targets -- -D warnings`
- Use `thiserror` for error types
- Use `tracing` macros for logging (not `println!`)

---

## Testing Strategy

**Unit Tests**
- Test individual functions/modules in isolation
- Mock external dependencies
- Fast execution (<1ms per test typically)

**Integration Tests**
- Test component interactions (e.g., adapter + executor)
- May use real network calls (bin/ tests on mainnet)
- Mark as `#[ignore]` if slow/flaky: `cargo test --ignored`

**Test Organization**
- Unit tests: `#[cfg(test)] mod tests` in same file
- Integration tests: `tests/` directory or `src/bin/` for mainnet validation
- Test count baseline: **202 tests** (as of Epic 2 completion)

---

## Git Workflow

```bash
# Before committing
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test

# Commit
git add .
git commit -m "feat: your feature description"
```

---

## Architecture Patterns

- **Modular Structure:** `adapters/{exchange}/{mod, config, types, signing, adapter}`
- **Trait-based:** `ExchangeAdapter` trait for all exchanges
- **Error Handling:** Return `ExchangeResult<T>` = `Result<T, ExchangeError>`
- **Async:** Use `tokio::join!` for parallel execution
- **Config:** Environment variables via `.env`, runtime config via YAML

---

## Questions?

Check the `_bmad-output/` artifacts for:
- Epic breakdown: `planning-artifacts/epics.md`
- Story implementation: `implementation-artifacts/*.md`
- Sprint status: `implementation-artifacts/sprint-status.yaml`
