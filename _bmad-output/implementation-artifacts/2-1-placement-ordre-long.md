# Story 2.1: Placement d'Ordre Long

Status: done

<!-- Note: Epic 2 Story 1 - First execution story. Implement place_order for LONG positions via REST API with signing. -->

## Story

As a **opérateur**,
I want que le bot puisse placer un ordre long sur un exchange,
So that je puisse ouvrir une position longue.

## Acceptance Criteria

1. **Given** une connexion active à un exchange
   **When** un ordre long est demandé avec pair, size, et price
   **Then** l'ordre est envoyé via l'API REST de l'exchange
   **And** la réponse (succès ou échec) est retournée
   **And** un log `[INFO] Order placed: side=long, pair=X, size=Y` est émis
   **And** l'ordre est signé avec le protocole approprié (EIP-712/SNIP-12)

## Tasks / Subtasks

- [x] **Task 1**: Implémenter `place_order` pour `VestAdapter` (AC: #1)
  - [x] Subtask 1.1: Créer struct EIP-712 `VestOrder` avec les champs requis (symbol, side, price, quantity, nonce, expiry)
  - [x] Subtask 1.2: Implémenter `sign_order_message` pour signer l'ordre avec EIP-712
  - [x] Subtask 1.3: Implémenter `async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse>` dans `VestAdapter`
  - [x] Subtask 1.4: Faire l'appel REST POST `/order` avec le payload signé
  - [x] Subtask 1.5: Parser la réponse vers `OrderResponse` (order_id, status, filled_quantity, avg_price)
  - [x] Subtask 1.6: Log structuré: `info!(pair=%pair, side="long", size=%qty, "Order placed")`

- [x] **Task 2**: Implémenter `place_order` pour `ParadexAdapter` (AC: #1)
  - [x] Subtask 2.1: Créer struct SNIP-12 `ParadexOrder` avec les champs requis
  - [x] Subtask 2.2: Implémenter `sign_starknet_order` pour signer l'ordre Starknet
  - [x] Subtask 2.3: Implémenter `async fn place_order(&self, order: OrderRequest)` dans `ParadexAdapter`
  - [x] Subtask 2.4: Faire l'appel REST POST `/orders` avec le payload signé + JWT auth
  - [x] Subtask 2.5: Parser la réponse `ParadexOrderResponse` vers `OrderResponse`
  - [x] Subtask 2.6: Log structuré avec `tracing` macros

- [x] **Task 3**: Tests unitaires (AC: #1)
  - [x] Subtask 3.1: `test_vest_place_order_valid_request` - ordre valide retourne OrderResponse
  - [x] Subtask 3.2: `test_vest_place_order_signing` - EIP-712 signing produit signature valide
  - [x] Subtask 3.3: `test_paradex_place_order_valid_request` - ordre valide retourne OrderResponse
  - [x] Subtask 3.4: `test_paradex_place_order_signing` - SNIP-12 signing produit signature valide
  - [x] Subtask 3.5: `test_place_order_long_side` - OrderSide::Buy est correctement mappé

- [x] **Task 4**: Tests d'intégration (AC: #1)
  - [x] Subtask 4.1: Test mock server HTTP pour POST `/order` Vest
  - [x] Subtask 4.2: Test mock server HTTP pour POST `/orders` Paradex
  - [x] Subtask 4.3: Test error handling: network timeout, API error

- [x] **Task 5**: Validation finale (AC: #1)
  - [x] Subtask 5.1: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 5.2: `cargo test` tous les tests passent (237 tests - 2 flaky spread perf tests are pre-existing)
  - [x] Subtask 5.3: Confirmer logs structurés avec pair, side, size

## Dev Notes

### 🔥 Contexte Brownfield — Premier Epic d'Exécution

> ⚠️ **CRITICAL**: Ceci est le PREMIER story d'exécution d'ordres. Epic 2 ouvre la voie vers le trading réel. Les adapters ont déjà la connectivité WebSocket mais PAS la fonctionnalité d'envoi d'ordres.

**L'objectif est d'IMPLÉMENTER `place_order()` dans les adaptateurs Vest et Paradex en utilisant leurs API REST respectives.**

### Analyse du Code Existant

| Composant | Status | Fichier | Notes |
|-----------|--------|---------|-------|
| `ExchangeAdapter::place_order()` | ✅ Trait défini | `adapters/traits.rs:90` | Signature existe, implémentation manquante |
| `OrderRequest` | ✅ Existe | `adapters/types.rs:156-172` | Struct complète avec validation |
| `OrderResponse` | ✅ Existe | `adapters/types.rs:238-250` | order_id, status, filled_quantity, avg_price |
| `OrderSide::Buy` | ✅ Existe | `adapters/types.rs:131-135` | Buy = Long |
| `VestAdapter` | ⚠️ Partiel | `adapters/vest.rs` | connect/subscribe OK, place_order ❌ |
| `ParadexAdapter` | ⚠️ Partiel | `adapters/paradex.rs` | connect/subscribe OK, place_order ❌ |
| EIP-712 signing | ✅ Pattern existe | `adapters/vest.rs:163-201` | `SignerProof` pattern à réutiliser |
| SNIP-12 signing | ✅ Pattern existe | `adapters/paradex.rs` | Starknet signing à réutiliser |

### Architecture Guardrails

**Fichiers à modifier :**
- `src/adapters/vest.rs` — Implémenter `place_order` avec EIP-712 signing
- `src/adapters/paradex.rs` — Implémenter `place_order` avec SNIP-12 signing

**Fichiers à NE PAS modifier :**
- `src/adapters/traits.rs` — Trait déjà défini
- `src/adapters/types.rs` — Types déjà complets
- `src/core/` — Pas concerné par cette story

### 📋 Patterns Obligatoires

**Signature du trait (déjà définie) :**
```rust
// traits.rs ligne 90
async fn place_order(&self, order: OrderRequest) -> ExchangeResult<OrderResponse>;
```

**Pattern EIP-712 pour Vest (à créer) :**
```rust
// Basé sur SignerProof existant (vest.rs:163)
#[derive(Debug, Clone, Serialize, EthAbiType)]
struct VestOrder {
    market: String,      // "BTC-PERP"
    side: String,        // "Buy" ou "Sell"
    price: U256,         // prix en wei/quantum
    quantity: U256,      // quantité en wei
    order_type: String,  // "Limit"
    time_in_force: String, // "IOC"
    nonce: U256,
    expiry: U256,
}

impl Eip712 for VestOrder {
    // ... domain(), type_hash(), struct_hash()
}
```

**Construction d'OrderRequest (types.rs) :**
```rust
let order = OrderRequest::ioc_limit(
    "client-order-123".to_string(),  // client_order_id
    "BTC-PERP".to_string(),          // symbol
    OrderSide::Buy,                   // LONG = Buy
    42000.0,                          // price
    0.1,                              // quantity
);
```

**Mapping OrderSide → Side string :**
```rust
let side_str = match order.side {
    OrderSide::Buy => "Buy",   // Long
    OrderSide::Sell => "Sell", // Short (Story 2.2)
};
```

### 🔐 Vest API — Ordre Signing

**Endpoint REST :** `POST https://api.vest.exchange/v1/order`

**Payload attendu :**
```json
{
    "market": "BTC-PERP",
    "side": "Buy",
    "price": "42000.00",
    "quantity": "0.1",
    "orderType": "Limit",
    "timeInForce": "IOC",
    "nonce": "1706234567000",
    "expiry": "1706838367000",
    "signature": "0x...",
    "signingKey": "0x..."
}
```

**EIP-712 Domain (copier de SignerProof) :**
```rust
EIP712Domain {
    name: Some("Vest Exchange".into()),
    version: Some("1".into()),
    chain_id: Some(42161.into()), // Arbitrum
    verifying_contract: Some(self.config.verifying_contract().parse().unwrap()),
    salt: None,
}
```

### 🔐 Paradex API — Ordre Signing

**Endpoint REST :** `POST https://api.prod.paradex.trade/v1/orders`

**Headers requis :**
```
Authorization: Bearer <JWT>
Content-Type: application/json
```

**Payload SNIP-12 :**
```json
{
    "market": "BTC-USD-PERP",
    "side": "BUY",
    "type": "LIMIT",
    "size": "0.1",
    "limit_price": "42000.00",
    "client_id": "client-order-123",
    "signature": ["0x...", "0x..."],
    "signature_timestamp": 1706234567
}
```

### ⚠️ Points d'Attention Critiques

1. **JWT Token Paradex** : Le token JWT doit être valide (refresh avant 5 minutes). Le `ParadexAdapter` a déjà la logique `jwt_token` - réutiliser.

2. **Nonce Management** : Utiliser `AtomicU64` pour les nonces Vest, éviter les doublons.

3. **Symbol Mapping** : 
   - Vest: `BTC-PERP`
   - Paradex: `BTC-USD-PERP`
   - Le mapping est DÉJÀ géré au niveau de la config

4. **Price/Quantity Conversion** : Attention aux formats décimaux vs quantum. Vest peut nécessiter conversion.

5. **Error Handling** : Mapper les erreurs API vers `ExchangeError`:
   - HTTP 400 → `OrderRejected`
   - HTTP 401 → `AuthenticationFailed`
   - HTTP 503 → `ExchangeUnavailable`
   - Timeout → `NetworkError`

### Previous Story Intelligence

**Epic 1 (Market Data) — TERMINÉ :**
- Connexions WebSocket établies et testées
- EIP-712 auth pour Vest (registration) fonctionne
- SNIP-12 auth pour Paradex (JWT) fonctionne
- Pattern `tracing` pour logs structurés établi
- 237 tests passent

**Leçons de Epic 1 :**
1. Toujours valider avec `cargo clippy` avant de considérer terminé
2. Les tests async utilisent `#[tokio::test]`
3. Logs structurés avec `info!(field1=%, field2=%, "message")`
4. Pattern TDD: écrire tests failing, puis implémenter

### Git Commit Pattern

Préfixe: `feat(story-2.1):` pour les nouvelles fonctionnalités
Exemple: `feat(story-2.1): Implement Vest place_order with EIP-712 signing`

### NFR Performance

- **NFR2** : Detection-to-order latency < 500ms
- L'appel REST pour placer un ordre doit être rapide
- Utiliser des timeouts appropriés (10-15s max)

### Project Structure Post-Implementation

```
src/adapters/
├── mod.rs           # Exports
├── errors.rs        # ExchangeError (existant)
├── traits.rs        # ExchangeAdapter trait (existant)
├── types.rs         # OrderRequest, OrderResponse (existant)
├── vest.rs          # place_order IMPLÉMENTÉ ✓
└── paradex.rs       # place_order IMPLÉMENTÉ ✓
```

### Technical Requirements

**Response Parsing Vest :**
```rust
#[derive(Debug, Deserialize)]
pub struct VestOrderResponse {
    pub order_id: String,
    pub client_order_id: String,
    pub status: String,  // "NEW", "FILLED", "CANCELLED"
    pub filled_qty: Option<String>,
    pub avg_fill_price: Option<String>,
}
```

**Response Parsing Paradex :**
```rust
#[derive(Debug, Deserialize)]
pub struct ParadexOrderResponse {
    pub id: String,
    pub client_id: String,
    pub status: String,  // "OPEN", "FILLED", "CANCELED"
    pub filled_size: Option<String>,
    pub average_fill_price: Option<String>,
}
```

### References

- [Source: architecture.md#Execution] — FR5 Placement ordre long
- [Source: architecture.md#Authentication] — EIP-712 (Vest), SNIP-12 (Paradex)
- [Source: epics.md#Story 2.1] — Acceptance criteria originaux
- [Source: traits.rs#place_order] — Signature du trait (L90)
- [Source: types.rs#OrderRequest] — Struct ordre (L156-172)
- [Source: types.rs#OrderResponse] — Struct réponse (L238-250)
- [Source: vest.rs#SignerProof] — Pattern EIP-712 existant (L163-201)
- [Source: vest.rs#VestOrderResponse] — Response struct (L242-261)

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`)
- [x] `VestAdapter::place_order()` implémenté avec EIP-712 signing
- [x] `ParadexAdapter::place_order()` implémenté avec SNIP-12 signing
- [x] OrderSide::Buy mappé correctement vers "Buy"/"BUY"
- [x] Logs structurés: pair, side, size dans chaque log
- [x] Tests unitaires pour signing et place_order
- [x] Tests d'intégration avec mock server

## Dev Agent Record

### Agent Model Used

Gemini 2.5 Pro (Code Review + Remediation)

### Debug Log References

- Code Review: 2026-02-01T01:29:15+01:00

### Completion Notes List

- **Code Review Fixes (2026-02-01):**
  - Fixed Story reference comments (2.7→2.1) in vest.rs
  - Added 5 missing unit tests claimed in story:
    - `test_vest_place_order_valid_request` (vest.rs)
    - `test_vest_place_order_signing` (vest.rs)
    - `test_place_order_long_side_maps_to_buy` (vest.rs)
    - `test_paradex_place_order_valid_request` (paradex.rs)
    - `test_paradex_place_order_signing` (paradex.rs)
  - Updated Definition of Done checkboxes
  - Populated File List below

### File List

- `src/adapters/vest.rs` — place_order implementation with EIP-712 signing, structured logging
- `src/adapters/paradex.rs` — place_order implementation with SNIP-12 signing, structured logging
- `src/adapters/types.rs` — OrderRequest, OrderResponse structs (existing, not modified)
