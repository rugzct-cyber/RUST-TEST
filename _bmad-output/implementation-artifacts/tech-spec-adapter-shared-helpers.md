---
title: 'Adapter Shared Helpers Refactoring'
slug: 'adapter-shared-helpers'
created: '2026-02-05T02:13:00+01:00'
status: 'complete'
stepsCompleted: [1, 2, 3, 4]
tech_stack: ['rust', 'tokio', 'native-tls', 'tokio-tungstenite', 'futures-util', 'rand']
files_to_modify:
  - Cargo.toml (ADD rand dependency)
  - src/adapters/shared/reconnect.rs (NEW)
  - src/adapters/shared/websocket.rs (NEW)
  - src/adapters/shared/mod.rs (NEW)
  - src/adapters/mod.rs
  - src/adapters/paradex/adapter.rs
  - src/adapters/vest/adapter.rs
code_patterns:
  - async-helper-functions
  - generic-parameters
  - exponential-backoff
test_patterns:
  - cargo-build
  - cargo-test
---

# Tech-Spec: Adapter Shared Helpers Refactoring

**Created:** 2026-02-05T02:13:00+01:00

## Overview

### Problem Statement

Les adapters Paradex et Vest contiennent 3 patterns dupliqués (~120 lignes au total):

1. **Reconnection Logic** (~75L): Exponential backoff identique (500ms * 2^attempt, cap 5000ms, max 3 attempts)
2. **TLS Connection** (~20L): Configuration TLS native quasi-identique (TLSv1.2 minimum)
3. **Stream Splitting** (~25L): Pattern de split WebSocket similaire (différence: Vest passe `last_pong`)

Cette duplication viole le principe DRY et augmente le risque d'incohérence lors de corrections de bugs.

### Solution

Créer un module `src/adapters/shared/` avec 3 helpers réutilisables:

1. `reconnect.rs::reconnect_with_backoff()` - Encapsule le loop de reconnexion paramétrique
2. `websocket.rs::connect_tls()` - Encapsule la création TLS + connexion WS
3. `websocket.rs::split_stream()` - Helper générique pour split et spawn reader

### Scope

**In Scope:**
- Création du module `src/adapters/shared/`
- Extraction des 3 patterns dupliqués
- Refactoring Paradex et Vest pour utiliser les helpers
- Maintien de la compatibilité comportementale existante

**Out of Scope:**
- Modification de la logique métier des readers (divergent trop entre adapters)
- Refactoring du `message_reader_loop` (rejeté dans investigation précédente)
- Changement des timeouts/constantes existants

## Context for Development

### Codebase Patterns

- Stage 11 a déjà centralisé `ConnectionHealth`, `next_subscription_id()`, `create_http_client()` dans `types.rs`
- Ce refactoring suit le même pattern: helpers partagés dans un module dédié
- Imports existants: `use crate::adapters::types::*`

### Files to Reference

| File | Purpose |
| ---- | ------- |
| `src/adapters/paradex/adapter.rs:1302-1376` | Paradex reconnect() - ~75 lignes |
| `src/adapters/vest/adapter.rs:1279-1343` | Vest reconnect() - ~65 lignes |
| `src/adapters/paradex/adapter.rs:300-320` | Paradex connect_websocket() TLS |
| `src/adapters/vest/adapter.rs:549-566` | Vest connect_websocket() TLS |
| `src/adapters/paradex/adapter.rs:376-405` | Paradex split_and_spawn_reader() |
| `src/adapters/vest/adapter.rs:617-640` | Vest split_and_spawn_reader() |
| `src/adapters/types.rs` | ConnectionState, ConnectionHealth |

### Technical Decisions

| # | Décision | Justification |
|---|----------|---------------|
| D1 | Helper A: Paramétrique via closures | `reconnect_with_backoff(connect_fn, on_success_fn)` pour rester générique |
| D2 | Helper B: Retourne `WebSocketStream` | Pas de stockage, l'adapter garde le contrôle de l'état |
| D3 | Helper C: **SKIP** | Stream split est trop couplé au reader spécifique (Vest a `last_pong`, Paradex a `order_tx`) - overhead de généricisation > bénéfice |
| D4 | Pas de changement aux constantes | MAX_RECONNECT_ATTEMPTS=3, backoff formula inchangée |
| D5 | **[RED TEAM V2]** Ajouter jitter anti-thundering herd | `backoff_ms + rand % 200` pour éviter reconnexions simultanées |
| D6 | **[RED TEAM V3]** Signature ownership-safe | Closures appelées séquentiellement, pas de capture `&mut self` simultanée |

## Implementation Plan

### Tasks

#### Task 0: Ajouter dépendance rand
**Fichier:** `Cargo.toml`

Ajouter sous `[dependencies]`:
```toml
rand = "0.8"
```

> **Justification:** Requis pour D5 (jitter anti-thundering herd) dans `reconnect_with_backoff()`.

---

#### Task 1: Créer le module shared avec helper TLS
**Fichier:** `src/adapters/shared/mod.rs` (NEW), `src/adapters/shared/websocket.rs` (NEW)

```rust
// websocket.rs
pub async fn connect_tls(url: &str) -> Result<WebSocketStream<...>, ExchangeError> {
    let tls = native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .map_err(|e| ExchangeError::ConnectionFailed(format!("TLS error: {}", e)))?;

    let (ws_stream, _) = connect_async_tls_with_config(url, None, false, Some(Connector::NativeTls(tls)))
        .await
        .map_err(|e| ExchangeError::WebSocket(Box::new(e)))?;

    Ok(ws_stream)
}
```

#### Task 2: Créer helper reconnect_with_backoff
**Fichier:** `src/adapters/shared/reconnect.rs` (NEW)

```rust
pub struct ReconnectConfig {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self { max_attempts: 3, initial_delay_ms: 500, max_delay_ms: 5000 }
    }
}

pub async fn reconnect_with_backoff<F, Fut>(
    config: ReconnectConfig,
    exchange_name: &str,
    mut connect_fn: F,
) -> ExchangeResult<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ExchangeResult<()>>,
{
    let mut last_error: Option<ExchangeError> = None;
    
    for attempt in 0..config.max_attempts {
        // V2 Fix: Jitter anti-thundering herd (0-199ms random)
        let jitter = rand::random::<u64>() % 200;
        let backoff_ms = std::cmp::min(
            config.initial_delay_ms * (1u64 << attempt),
            config.max_delay_ms
        ) + jitter;
        tracing::info!(
            "{}: Reconnect attempt {} of {}, waiting {}ms...",
            exchange_name, attempt + 1, config.max_attempts, backoff_ms
        );
        
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        
        match connect_fn().await {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!("{}: Reconnect attempt {} failed: {}", exchange_name, attempt + 1, e);
                last_error = Some(e);
            }
        }
    }
    
    Err(last_error.unwrap_or_else(|| 
        ExchangeError::ConnectionFailed("Reconnection failed after max attempts".into())
    ))
}
```

#### Task 3: Mettre à jour src/adapters/mod.rs
Ajouter: `pub mod shared;`

#### Task 4: Refactorer Paradex connect_websocket()
Remplacer L304-317 par appel à `shared::websocket::connect_tls(url)`.

#### Task 5: Refactorer Vest connect_websocket()
Remplacer L554-562 par appel à `shared::websocket::connect_tls(url)`.

#### Task 6: Refactorer Paradex reconnect()
Utiliser `reconnect_with_backoff()` avec gestion d'état ConnectionState en wrapper.

#### Task 7: Refactorer Vest reconnect()
Utiliser `reconnect_with_backoff()` avec gestion d'état ConnectionState en wrapper.

### Acceptance Criteria

```gherkin
AC1: TLS Helper fonctionne
Given le module shared/websocket.rs existe
When connect_tls(url) est appelé avec une URL valide
Then une WebSocketStream connectée est retournée

AC2: Reconnect Helper fonctionne  
Given le module shared/reconnect.rs existe
When reconnect_with_backoff() est appelé avec une fonction de connexion
Then le backoff exponentiel est appliqué (500ms, 1000ms, 2000ms)
And max 3 tentatives sont effectuées

AC3: Adapters utilisent les helpers
Given Paradex et Vest sont refactorés
When cargo build est exécuté
Then 0 erreurs et 0 warnings

AC4: Comportement préservé
Given les adapters refactorés
When cargo test est exécuté
Then tous les tests existants passent
```

## Additional Context

### Dependencies

- `rand = "0.8"` (**NEW** - pour jitter anti-thundering herd D5)
- `native-tls` (déjà présent)
- `tokio-tungstenite` avec feature `native-tls` (déjà présent)
- `futures-util` pour `StreamExt::split()` (déjà présent)

### Testing Strategy

**Vérification automatisée:**
```bash
cargo build 2>&1 | head -50    # AC3: 0 erreurs
cargo test 2>&1 | tail -20     # AC4: tous tests passent
```

**Tests existants à valider:**
- `src/adapters/types.rs` tests unitaires (test_exponential_backoff_timing)
- `src/adapters/paradex/adapter.rs` tests (test_warm_up_http_makes_request)
- `src/adapters/vest/adapter.rs` tests (test_warm_up_http_makes_request)

### Notes

**Estimation LOC économisées:**
- Helper A (reconnect): ~50 lignes (déduplique ~70% du code reconnect)
- Helper B (TLS): ~16 lignes (déduplique ~95% du code TLS)
- Helper C: **SKIP** (ROI insuffisant)
- **Total: ~66 LOC** (révision à la baisse vs estimation initiale de 120L)

**Révision Pattern C:** Après analyse approfondie, le split_and_spawn_reader diverge trop:
- Paradex passe: `ws_receiver, shared_orderbooks, last_data`
- Vest passe: `ws_receiver, shared_orderbooks, last_pong, last_data`
- La généricisation nécessiterait un trait ou des closures complexes avec overhead > bénéfice.
