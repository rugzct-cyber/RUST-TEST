---
stepsCompleted: [1, 2, 3, 4, 5, 6, 7, 8]
inputDocuments:
  - product-brief-bot4-2026-01-31.md
  - prd.md
  - docs/index.md
  - docs/architecture.md
  - docs/data-models.md
  - docs/api-contracts.md
  - docs/source-tree.md
workflowType: 'architecture'
project_name: 'bot4'
user_name: 'rugz'
date: '2026-01-31'
lastStep: 8
status: 'complete'
completedAt: '2026-01-31'
---

# Architecture Decision Document

_This document builds collaboratively through step-by-step discovery. Sections are appended as we work through each architectural decision together._

---

## Project Context Analysis

### Requirements Overview

**Functional Requirements (21 FRs):**

| Catégorie | FRs | Implications Architecturales |
|-----------|-----|------------------------------|
| **Market Data** | FR1-4 | WebSocket dual connect, async message handlers, orderbook state management |
| **Execution** | FR5-9 | Order execution engine, retry logic, atomic rollback pattern |
| **State Management** | FR10-12 | Supabase persistence layer, in-memory cache sync |
| **Configuration** | FR13-15 | YAML loader + env var injection, validation rules |
| **Resilience** | FR16-18 | Reconnection state machine, graceful shutdown handler |
| **Observability** | FR19-21 | Structured tracing, credential redaction middleware |

**Non-Functional Requirements (14 NFRs):**

| Catégorie | Target | Architecture Driver |
|-----------|--------|---------------------|
| **Performance** | Spread <2ms, Execution <500ms | Zero-copy parsing, inline functions, Tokio async |
| **Security** | Credentials jamais en clair | `SanitizedValue` pattern, WSS only |
| **Reliability** | 99% uptime, <5s reconnect | Health monitoring, circuit breaker |
| **Integration** | Vest + Paradex + Supabase | Unified `ExchangeAdapter` trait |

**Scale & Complexity:**

- **Primary domain:** Backend Trading Bot (Rust/Tokio)
- **Complexity level:** **High** (HFT, multi-protocol auth, real-time)
- **Estimated architectural components:** 6 (Adapters, Core, Config, State, Channels, Runtime)

### Technical Constraints & Dependencies

| Contrainte | Impact |
|------------|--------|
| **Brownfield codebase** | ~8,900 lignes existantes à maintenir/refactorer |
| **v3 cleanup requis** | Pattern "scout" et code résiduel à supprimer |
| **Single pair MVP** | Architecture simplifiée, multi-pair en Phase 4 |
| **Supabase dépendance** | Position state persistence externe |
| **Dual auth protocols** | EIP-712 (Vest) + SNIP-12 (Paradex) |

### Cross-Cutting Concerns Identified

1. **Error Propagation** — `thiserror` + `anyhow` hierarchy unifié
2. **Credential Security** — Redaction automatique dans tous les logs
3. **Connection Health** — Monitoring unifié pour tous les adapters
4. **Graceful Shutdown** — Broadcast channel pour coordination des tâches
5. **Configuration Validation** — Règles appliquées au chargement YAML

---

## Starter Template Evaluation

### Primary Technology Domain

**Backend Trading Bot (Rust/Tokio)** — HFT arbitrage system with real-time WebSocket connections

### Brownfield Context Assessment

Ce projet est **brownfield** avec une codebase existante de ~8,900 lignes Rust. L'évaluation de starter template s'adapte en conséquence.

### Approach Selected: Refactor In-Place

**Rationale:**
- Structure modulaire existante (adapters/, core/, config/) est solide
- Code core fonctionnel et testé (SpreadCalculator, VWAP, ExchangeAdapter)
- Cleanup ciblé requis (pattern "scout", résidus v3) — pas de rewrite complet
- Évite la perte de code fonctionnel (5,000+ lignes d'adapters exchange)
- Leçon apprise de bot3 : éviter la sur-ingénierie

### Existing Technology Stack (Retained)

| Category | Technology | Status |
|----------|------------|--------|
| **Language** | Rust 2021 Edition | ✅ Conserver |
| **Runtime** | Tokio (async, full features) | ✅ Conserver |
| **HTTP** | reqwest (json features) | ✅ Conserver |
| **WebSocket** | tokio-tungstenite (native-tls) | ✅ Conserver |
| **Crypto EVM** | ethers (legacy features) | ✅ Conserver |
| **Crypto Stark** | starknet-crypto | ✅ Conserver |
| **Serialization** | serde, serde_yaml, serde_json | ✅ Conserver |
| **Error Handling** | thiserror, anyhow | ✅ Conserver |
| **Logging** | tracing, tracing-subscriber (json, env-filter) | ✅ Conserver |
| **Database** | Supabase (external) | ✅ Conserver |

### Cleanup Required (Phase 0)

| Item | Action |
|------|--------|
| Pattern "scout" | Supprimer — inexistant dans MVP |
| Intentions v3 | Supprimer — non implémentées |
| Code mort | Identifier et supprimer |
| Flow simplifié | Refactorer vers execution directe |

### Initialization Command

```bash
# N/A - Projet brownfield existant
# Cleanup command suggérée:
cargo clippy --all-targets -- -D warnings
cargo test
```

---

## Core Architectural Decisions

### Decision Priority Analysis

**Critical Decisions (Block Implementation):**
- Runtime flow : Multi-Task Pipeline avec channels
- Failed leg handling : Auto-close (NFR7 compliance)
- Auth protocols : EIP-712 + SNIP-12 (exchange requirements)

**Important Decisions (Shape Architecture):**
- Data schema minimal, extensible
- Fixed retry logic
- JSON structured logging

**Deferred Decisions (Post-MVP):**
- VPS deployment (Phase 4)
- Prometheus metrics (Phase 3)
- DashMap concurrence (si multi-pair)

### Data Architecture

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Positions Schema** | Minimal | MVP focus, extensible en Phase 2 |
| **Cache Strategy** | HashMap<String, Orderbook> | Single-pair, pas de concurrence complexe |
| **Persistence** | Supabase (external) | Existant, FR10-12 |

### Runtime Architecture

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Execution Flow** | Multi-Task Pipeline | Séparation des concerns, channels existants |
| **Task Coordination** | select! + broadcast channel | Explicite, pattern Tokio standard |
| **Shutdown** | Graceful via broadcast | FR17 compliance |

### Resilience Patterns

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Retry Logic** | Fixed (3 attempts, fixed delay) | Simple, prévisible pour MVP |
| **Failed Leg** | Auto-close opposite | NFR7: no single-leg exposure |
| **Reconnection** | Immediate with backoff | NFR9: <5s reconnect |

### Deployment Strategy

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Environment** | Local only (MVP) | VPS prévu Phase 4 |
| **Logging** | JSON stdout | FR19 compliance, log aggregation ready |
| **Monitoring** | Logs only | Dashboard prévu Phase 3 |

### Decision Impact Analysis

**Implementation Sequence:**
1. Phase 0: Cleanup v3 code
2. Phase 1: Multi-task pipeline setup
3. Phase 1: Retry + auto-close logic
4. Phase 1: Supabase position sync

**Cross-Component Dependencies:**
- Channels → tous les composants (spread monitor, execution, adapters)
- Auto-close → nécessite order tracking dans execution engine

---

## Implementation Patterns & Consistency Rules

### Naming Patterns

**Rust Standard (appliqué) :**

| Element | Convention | Example |
|---------|------------|---------|
| **Modules** | `snake_case` | `adapters`, `spread`, `vwap` |
| **Functions** | `snake_case` | `calculate_spread()`, `place_order()` |
| **Structs/Enums** | `PascalCase` | `Orderbook`, `SpreadResult`, `ExchangeError` |
| **Constants** | `SCREAMING_SNAKE_CASE` | `MAX_RETRIES`, `PING_TIMEOUT_SECS` |
| **Type aliases** | `PascalCase` | `ExchangeResult<T>`, `SharedAppState` |

### Error Handling Patterns

**Pattern établi :**
```rust
// Custom errors avec thiserror
#[derive(Debug, thiserror::Error)]
pub enum ExchangeError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
}

// Propagation avec ?
async fn connect(&mut self) -> ExchangeResult<()> {
    self.ws.connect().await?;
    Ok(())
}
```

**Règles :**
- ✅ Utiliser `?` pour propagation
- ✅ `thiserror` pour les erreurs custom
- ❌ Éviter `.unwrap()` sauf dans les tests
- ❌ Pas de `panic!` en production

### Logging Patterns

**Format établi :**
```rust
// Événements business
info!(pair = %pair, spread = spread_pct, "Spread detected");

// Erreurs avec contexte
error!(exchange = %name, error = ?e, "Connection failed");

// Debug pour troubleshooting
debug!(orderbook = ?ob, "Orderbook updated");
```

**Niveaux de log :**
- `info!` → Événements business (trades, spread détection)
- `warn!` → Situations anormales récupérables
- `error!` → Échecs nécessitant attention
- `debug!` → Détails pour troubleshooting
- `SanitizedValue` pour credentials

### Async Patterns

**Pattern établi :**
```rust
// Task avec shutdown
tokio::select! {
    _ = shutdown_rx.recv() => break,
    msg = ws_rx.recv() => handle(msg),
}

// Channels pour communication
let (tx, rx) = mpsc::channel::<SpreadOpportunity>(100);
```

**Règles :**
- `select!` avec branche shutdown en premier
- Channels typés pour communication inter-task
- `broadcast` pour signals (shutdown)
- `mpsc` pour data flow

### Struct Patterns

**Dérivés standard :**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Orderbook { ... }

#[derive(Debug)]  // Minimum pour logging
pub struct VestAdapter { ... }
```

### Testing Patterns

**Organisation :**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spread_calculation() { ... }

    #[tokio::test]
    async fn test_async_connect() { ... }
}
```

**Règles :**
- Tests dans le même fichier (module `tests`)
- Préfixe `test_` pour les noms
- `#[tokio::test]` pour async

### Enforcement Guidelines

**Tous les agents AI DOIVENT :**
1. Exécuter `cargo clippy` avant commit
2. Suivre `rustfmt` formatting
3. Utiliser les patterns d'erreur `thiserror`
4. Logger avec `tracing` macros

---

## Project Structure & Boundaries

### Complete Project Directory Structure

```
bot4/
├── Cargo.toml                    # Package manifest
├── Cargo.lock                    # Dependency lock
├── config.yaml                   # Bot configuration (runtime)
├── .env                          # Environment secrets (gitignored)
├── .env.example                  # Template for .env
├── README.md                     # Project documentation
├── docs/                         # Auto-generated documentation
│   ├── index.md                  # Documentation index
│   ├── architecture.md           # Architecture overview
│   ├── api-contracts.md          # API documentation
│   ├── data-models.md            # Data structures
│   └── source-tree.md            # Source organization
└── src/
    ├── main.rs                   # Entry point, runtime setup
    ├── lib.rs                    # Module exports
    ├── error.rs                  # Unified error types
    ├── adapters/                 # Exchange adapters (~5,000 lines)
    │   ├── mod.rs                # Adapter exports
    │   ├── common.rs             # ExchangeAdapter trait
    │   ├── vest/                 # Vest exchange integration
    │   │   ├── mod.rs
    │   │   ├── adapter.rs        # VestAdapter implementation
    │   │   ├── auth.rs           # EIP-712 signing
    │   │   ├── orderbook.rs      # Orderbook parsing
    │   │   └── types.rs          # Vest-specific types
    │   └── paradex/              # Paradex exchange integration
    │       ├── mod.rs
    │       ├── adapter.rs        # ParadexAdapter implementation
    │       ├── auth.rs           # SNIP-12 signing
    │       ├── orderbook.rs      # Orderbook parsing
    │       └── types.rs          # Paradex-specific types
    ├── core/                     # Business logic (~2,500 lines)
    │   ├── mod.rs                # Core exports
    │   ├── spread.rs             # SpreadCalculator
    │   ├── vwap.rs               # VWAP engine
    │   ├── channels.rs           # ChannelBundle definitions
    │   ├── runtime.rs            # Task orchestration
    │   └── state.rs              # [NEW] Supabase persistence
    └── config/                   # Configuration (~500 lines)
        ├── mod.rs
        └── loader.rs             # YAML + env var injection
```

### Architectural Boundaries

**API Boundaries:**

| Boundary | Files | Protocol |
|----------|-------|----------|
| Vest Exchange | `adapters/vest/adapter.rs` | WebSocket (WSS) + REST |
| Paradex Exchange | `adapters/paradex/adapter.rs` | WebSocket (WSS) + REST |
| Supabase | `core/state.rs` | REST (HTTPS) |

**Component Boundaries:**

| Component | Responsibility | Communicates With |
|-----------|----------------|-------------------|
| `VestAdapter` | Vest connectivity | `ChannelBundle` |
| `ParadexAdapter` | Paradex connectivity | `ChannelBundle` |
| `SpreadCalculator` | Spread detection | Receives from channels |
| `Runtime` | Task orchestration | Spawns all tasks |

**Data Boundaries:**

| Data | Source | Consumer |
|------|--------|----------|
| Orderbooks | Adapters | SpreadCalculator |
| SpreadOpportunity | SpreadCalculator | Execution logic |
| PositionState | Supabase | Persistence layer |

### Requirements to Structure Mapping

| FR Category | Primary Files |
|-------------|---------------|
| **Market Data (FR1-4)** | `adapters/*/orderbook.rs`, `core/channels.rs` |
| **Execution (FR5-9)** | `adapters/*/adapter.rs` (place_order) |
| **State (FR10-12)** | `core/state.rs` [NEW] |
| **Config (FR13-15)** | `config/loader.rs` |
| **Resilience (FR16-18)** | `core/runtime.rs`, `adapters/common.rs` |
| **Observability (FR19-21)** | All files (tracing macros) |

### Integration Points

**Internal Communication:**
```
Adapters → mpsc::Sender<Orderbook> → SpreadCalculator
SpreadCalculator → mpsc::Sender<SpreadOpportunity> → Executor
Runtime → broadcast::Sender<()> → All tasks (shutdown)
```

**External Integrations:**

| Service | Purpose | Connection |
|---------|---------|------------|
| Vest API | Market data + orders | WSS + REST |
| Paradex API | Market data + orders | WSS + REST |
| Supabase | Position persistence | REST (postgrest) |

### Files to Create (Phase 1)

| File | Purpose | Priority |
|------|---------|----------|
| `src/core/state.rs` | Supabase position sync | High |
| `src/core/execution.rs` | Order execution logic | High |

---

## Architecture Validation Results

### Coherence Validation ✅

**Decision Compatibility:**
All technology choices work together without conflicts. Rust 2021 + Tokio async runtime + WebSocket libraries form a cohesive stack. Authentication protocols (EIP-712, SNIP-12) are isolated in adapter modules.

**Pattern Consistency:**
Implementation patterns (naming, error handling, logging, async) are aligned with Rust standard conventions and the existing codebase. All patterns support the architectural decisions made.

**Structure Alignment:**
Project structure (adapters/, core/, config/) directly supports the modular architecture. Component boundaries are clear and enable independent development.

### Requirements Coverage ✅

**Functional Requirements Coverage:**

| FR Category | Status | Architectural Support |
|-------------|--------|----------------------|
| Market Data (FR1-4) | ✅ | adapters/*/orderbook.rs, core/channels.rs |
| Execution (FR5-9) | ✅ | adapters/*/adapter.rs (place_order) |
| State Management (FR10-12) | ⚠️ | core/state.rs [to create] |
| Configuration (FR13-15) | ✅ | config/loader.rs |
| Resilience (FR16-18) | ✅ | core/runtime.rs, reconnect patterns |
| Observability (FR19-21) | ✅ | tracing macros throughout |

**Non-Functional Requirements Coverage:**

| NFR | Status | How Addressed |
|-----|--------|---------------|
| Performance (<2ms spread) | ✅ | Tokio async, zero-copy patterns |
| Security (credentials) | ✅ | SanitizedValue, WSS only |
| Reliability (99% uptime) | ✅ | Auto-close, reconnect backoff |
| Integration (dual exchange) | ✅ | ExchangeAdapter trait |

### Implementation Readiness ✅

**Decision Completeness:**
- All critical decisions documented with technology choices
- Implementation patterns comprehensive for Rust development
- Consistency rules enforceable via cargo clippy + rustfmt

**Structure Completeness:**
- Complete directory structure defined
- All files and directories specified
- Integration points clearly mapped

**Pattern Completeness:**
- Naming conventions established (Rust standard)
- Communication patterns specified (channels)
- Process patterns documented (error handling, logging)

### Gap Analysis

| Gap | Priority | Resolution |
|-----|----------|------------|
| `core/state.rs` missing | 🔴 High | Create in Phase 1 |
| `core/execution.rs` missing | 🔴 High | Create in Phase 1 |
| Unit test coverage limited | 🟡 Medium | Extend progressively |
| Integration tests absent | 🟡 Medium | Add in Phase 2 |

### Architecture Completeness Checklist

**✅ Requirements Analysis**
- [x] Project context thoroughly analyzed
- [x] Scale and complexity assessed (High)
- [x] Technical constraints identified (brownfield, dual auth)
- [x] Cross-cutting concerns mapped

**✅ Architectural Decisions**
- [x] Critical decisions documented
- [x] Technology stack fully specified
- [x] Integration patterns defined
- [x] Performance considerations addressed

**✅ Implementation Patterns**
- [x] Naming conventions established
- [x] Structure patterns defined
- [x] Communication patterns specified
- [x] Process patterns documented

**✅ Project Structure**
- [x] Complete directory structure defined
- [x] Component boundaries established
- [x] Integration points mapped
- [x] Requirements to structure mapping complete

### Architecture Readiness Assessment

**Overall Status:** ✅ READY FOR IMPLEMENTATION

**Confidence Level:** High — Structure solide, décisions cohérentes, patterns établis

**Key Strengths:**
- Existing functional codebase (~8,900 lines)
- Well-defined exchange adapters (Vest, Paradex)
- Modular architecture supporting brownfield refactor
- Clear separation of concerns

**Areas for Future Enhancement:**
- Add comprehensive test suite
- Implement execution engine
- Add Supabase persistence layer
- Consider VPS deployment (Phase 4)

### Implementation Handoff

**AI Agent Guidelines:**
1. Follow all architectural decisions exactly as documented
2. Use implementation patterns consistently across all components
3. Respect project structure and boundaries
4. Refer to this document for all architectural questions
5. Run `cargo clippy` and `cargo test` before commits

**First Implementation Priority:**
```bash
# Phase 0: Cleanup
cargo clippy --all-targets -- -D warnings
# Identify and remove v3/scout code

# Phase 1: Core implementation
# 1. Create src/core/state.rs
# 2. Create src/core/execution.rs
# 3. Wire multi-task pipeline in runtime.rs
```
