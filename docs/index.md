# HFT Arbitrage Bot - Documentation Index

> **Project:** bot4 (HFT Arbitrage Bot V1)  
> **Type:** Rust Backend (Tokio)  
> **Updated:** 2026-02-04 (Exhaustive Rescan)

---

## Quick Navigation

| Document | Purpose |
|----------|---------|
| [Architecture](architecture.md) | System design, module structure, data flow |
| [Data Models](data-models.md) | All structs, enums, type definitions |
| [API Contracts](api-contracts.md) | Trait interfaces, REST/WS protocols |
| [Source Tree](source-tree.md) | File listing with sizes and purposes |

---

## Project Overview

Delta-neutral HFT arbitrage bot for perpetual futures. Monitors price spreads between Vest and Paradex DEXes, executing simultaneous long/short positions when spreads exceed thresholds.

### V1 Architecture Highlights

- **Lock-Free Design:** `Arc<RwLock>` shared orderbooks, no Mutex in hot path
- **40Hz Polling:** 25ms monitoring intervals
- **Parallel Execution:** `tokio::join!` for simultaneous order placement
- **No Persistence:** Pure HFT mode, no Supabase

### Technology Stack

| Category | Technology |
|----------|------------|
| Language | Rust 2021 |
| Runtime | Tokio (async) |
| Transport | WebSocket + REST |
| Crypto | ethers (EIP-712), starknet-crypto (SNIP-12) |
| Config | YAML (serde_yaml) |
| Logging | tracing |

---

## Codebase Statistics

| Module | Files | Size | Description |
|--------|-------|------|-------------|
| adapters/vest | 5 | 88KB | Vest Markets (EIP-712 auth) |
| adapters/paradex | 5 | 106KB | Paradex (Starknet auth) |
| adapters/common | 4 | 36KB | Shared types and traits |
| core | 6 | 97KB | Business logic |
| config | 4 | 22KB | Configuration |
| **Total** | **27** | **~360KB** | |

---

## Module Summary

```
src/
├── adapters/    # Exchange connectivity
│   ├── vest/        → EIP-712 signing, REST/WS
│   ├── paradex/     → SNIP-12 signing, REST/WS
│   ├── traits.rs    → ExchangeAdapter interface
│   └── types.rs     → Orderbook, Order types
│
├── core/        # Business logic
│   ├── execution.rs → DeltaNeutralExecutor
│   ├── spread.rs    → SpreadCalculator
│   ├── runtime.rs   → execution_task (with exit monitoring)
│   ├── monitoring.rs→ monitoring_task (40Hz polling)
│   └── channels.rs  → SpreadOpportunity channel
│
└── config/      # Configuration
    ├── types.rs     → BotConfig, AppConfig
    └── loader.rs    → YAML loading
```

---

## Getting Started

### Prerequisites

- Rust 1.70+
- Exchange API credentials (Vest, Paradex)

### Configuration

1. Create `config.yaml` with bot settings:
   ```yaml
   bots:
     - id: "btc-arb"
       pair: BTC-PERP
       dex_a: vest
       dex_b: paradex
       spread_entry: 0.15
       spread_exit: 0.05
       leverage: 10
       position_size: 0.01
   ```

2. Set environment variables in `.env`:
   ```bash
   VEST_PRIMARY_ADDR=0x...
   VEST_PRIMARY_KEY=0x...
   VEST_SIGNING_KEY=0x...
   PARADEX_PRIVATE_KEY=0x...
   PARADEX_ACCOUNT_ADDRESS=0x...
   ```

### Running

```bash
cargo run --release
```

### Testing

```bash
cargo test
```

---

## AI Development Context

Key patterns for AI-assisted development:

1. **ExchangeAdapter Trait** - Uniform interface for adding exchanges
2. **SharedOrderbooks** - Thread-safe via `Arc<RwLock<HashMap>>`
3. **Channel Pipeline** - `mpsc` for spread opportunities
4. **Error Propagation** - `thiserror` + `anyhow`
5. **Config Validation** - Compile-time + runtime rules

---

## Related Resources

- [README.md](../README.md) - Project introduction
- [CONTRIBUTING.md](../CONTRIBUTING.md) - Contribution guide
- [Cargo.toml](../Cargo.toml) - Dependencies
