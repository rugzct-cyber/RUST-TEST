# Story 1.2: Connexion WebSocket Paradex

Status: review

<!-- Note: Epic 1 Story 2 - Establishes Paradex WebSocket foundation, mirroring Story 1.1 pattern for Vest. -->

## Story

As a **opérateur**,
I want que le bot se connecte au WebSocket de Paradex,
So that je puisse recevoir les données de marché en temps réel.

## Acceptance Criteria

1. **Given** les credentials Paradex configurés dans `.env`
   **When** le bot démarre
   **Then** une connexion WebSocket WSS est établie avec Paradex
   **And** un log `[INFO] Paradex WebSocket connected` est émis
   **And** le bot gère l'authentification SNIP-12

## Tasks / Subtasks

- [x] **Task 1**: Valider la configuration Paradex existante (AC: #1)
  - [x] Subtask 1.1: Vérifier que `ParadexConfig::from_env()` charge correctement les credentials
  - [x] Subtask 1.2: Confirmer que les variables `.env` requises sont documentées
  - [x] Subtask 1.3: Écrire un test unitaire validant le chargement de config (pattern Story 1.1)

- [x] **Task 2**: Valider la connexion WebSocket WSS (AC: #1)
  - [x] Subtask 2.1: Localiser la fonction de connexion WebSocket dans `paradex.rs`
  - [x] Subtask 2.2: Valider que la connexion utilise `wss://` (NFR6)
  - [x] Subtask 2.3: Valider le handshake WebSocket existant
  - [x] Subtask 2.4: Écrire un test d'intégration pour la connexion

- [x] **Task 3**: Valider l'authentification SNIP-12 (AC: #1)
  - [x] Subtask 3.1: Valider `sign_auth_message` function et typed data structure
  - [x] Subtask 3.2: Vérifier le flow de signature avec private key Starknet
  - [x] Subtask 3.3: Confirmer l'envoi du message d'authentification (`build_ws_auth_message`)
  - [x] Subtask 3.4: Écrire un test unitaire pour la génération de signature SNIP-12

- [x] **Task 4**: Émettre le log de connexion (AC: #1)
  - [x] Subtask 4.1: Ajouter `info!(exchange = "paradex", "Paradex WebSocket connected")` après succès
  - [x] Subtask 4.2: Utiliser `tracing` macros avec contexte (exchange name)

- [x] **Task 5**: Validation finale (AC: #1)
  - [x] Subtask 5.1: `cargo clippy --all-targets -- -D warnings` sans erreurs
  - [x] Subtask 5.2: `cargo test` - tous les tests passent
  - [ ] Subtask 5.3: Test manuel de connexion avec credentials valides
    > **Note:** Test manuel peut être différé — `main.rs` est un scaffold MVP.

## Dev Notes

### Contexte Brownfield — Code Existant

> ⚠️ **CRITICAL**: Ce projet est brownfield avec ~8,900 lignes existantes. Le fichier `src/adapters/paradex.rs` contient déjà **2,878 lignes** d'implémentation.

**L'objectif n'est PAS de réécrire mais de VALIDER et COMPLÉTER le code existant.**

### Analyse du Code Existant

Le fichier `src/adapters/paradex.rs` contient déjà :

| Composant | Status | Lignes |
|-----------|--------|--------|
| `ParadexConfig` | ✅ Existe | 62-69 |
| `ParadexConfig::from_env()` | ✅ Existe | 72-88 |
| `ParadexConfig::ws_base_url()` | ✅ Existe | 99-106 |
| `sign_auth_message()` (SNIP-12) | ✅ Existe | 372-484 |
| `build_ws_auth_message()` | ✅ Existe | 338-348 |
| `build_ws_url()` | ✅ Existe | 350-357 |
| `ParadexOrderbookData` + parsing | ✅ Existe | 217-277 |

### Architecture Guardrails

**Fichiers à modifier :**
- `src/adapters/paradex.rs` — adapter principal (validation/complétion)
- `src/adapters/mod.rs` — vérifier exports

**Fichiers à NE PAS modifier :**
- `src/core/` — pas de changements core pour cette story
- `src/config/` — config loader reste identique
- `src/adapters/vest.rs` — déjà validé dans Story 1.1

**Patterns obligatoires (copiés de Story 1.1) :**
```rust
// Logging avec tracing
info!(exchange = "paradex", "Paradex WebSocket connected");

// Erreurs avec thiserror
#[derive(Debug, thiserror::Error)]
pub enum ParadexError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
}

// Async avec tokio
async fn connect(&mut self) -> ExchangeResult<()> {
    // ...
}
```

### Project Structure Notes

**Structure actuelle :**
```
src/adapters/
├── mod.rs           # Exports
├── vest.rs          # 2,140 lignes — adapter Vest (Story 1.1 ✅)
├── paradex.rs       # 2,878 lignes — adapter Paradex (cette story)
├── traits.rs        # ExchangeAdapter trait
├── types.rs         # Types partagés
└── errors.rs        # Erreurs adapters
```

**Dépendances inter-modules :**
- `config` ← `core` ← `adapters` (unidirectionnel)
- `adapters` importe `config` pour credentials
- `adapters` n'importe PAS `core` directement

### Technical Requirements

**Stack technologique (à conserver) :**
- `tokio-tungstenite` pour WebSocket (native-tls)
- `starknet-crypto` pour signatures SNIP-12 (Starknet)
- `tracing` pour logging structuré
- `serde` pour serialization

**Variables d'environnement requises :**
```env
PARADEX_PRIVATE_KEY=0x...        # Clé privée Starknet (felt252)
PARADEX_ACCOUNT_ADDRESS=0x...    # Adresse compte Starknet
PARADEX_PRODUCTION=true|false    # Mode testnet/prod
```

**NFRs applicables :**
- NFR6: WSS (TLS) uniquement — pas de WS non sécurisé
- NFR4: Private keys jamais en clair dans logs — utiliser `SanitizedValue`
- NFR13: Paradex API compatible avec version actuelle

### SNIP-12 Authentication Details

**Flow d'authentification Paradex :**
1. Générer timestamp et expiration
2. Signer typed data avec `sign_auth_message()` → (r, s)
3. Appeler `/auth` REST endpoint → JWT token
4. Connecter WebSocket et envoyer `build_ws_auth_message(jwt)` pour auth

**Différence avec EIP-712 (Vest) :**
| Aspect | Vest (EIP-712) | Paradex (SNIP-12) |
|--------|----------------|-------------------|
| Crypto | `ethers` / secp256k1 | `starknet-crypto` / pedersen |
| Signing | ECDSA | Pedersen hash + ECDSA on StarkCurve |
| Format | `SignerProof` struct | (signature_r, signature_s) tuple |

### Previous Story Intelligence

**Story 1.1 terminée (Connexion WebSocket Vest) :**
- Validation du code existant au lieu de rewrite
- Ajout du log `info!(exchange = "vest", "Vest WebSocket connected")`
- Tests `test_vest_config_from_env` et `test_vest_config_from_env_missing_required`
- Pattern `#[serial(env)]` pour tests touchant env vars

**Leçons apprises :**
1. Utiliser `#[serial(env)]` pour tests touchant env vars
2. Vérifier que le log de connexion est bien émis après handshake réussi
3. Le test manuel peut être différé car `main.rs` est scaffold

### Git Intelligence Summary

**5 derniers commits :**
1. `2ba43e7` — Story 1.1 code review complete - marked done
2. `3f3e571` — feat(story-1.1): Add Vest WebSocket connection log and from_env tests
3. `f9f5da7` — fix flaky tests Story 0.3, Hash derives
4. `54ba74a` — refactor glob exports → explicit re-exports
5. `8a4765c` — Story 0.2 complete, Dev Agent Record

**Pattern établi :** Commits préfixés par type (`feat`, `fix`, `docs`)

### Latest Tech Information

**tokio-tungstenite (current):**
- WebSocket async avec native-tls
- Pattern: `connect_async(url)` retourne `(WebSocketStream, Response)`

**starknet-crypto (SNIP-12):**
- Pedersen hash pour typed data hashing
- ECDSA sur StarkCurve pour signature
- `FieldElement` pour représentation des valeurs Starknet

**Paradex API specifics:**
- JWT token lifetime: 5 minutes (constante `JWT_LIFETIME_MS = 300_000`)
- Refresh recommandé à 3 minutes (`JWT_REFRESH_BUFFER_MS = 120_000`)
- JSON-RPC 2.0 pour WebSocket messages

### References

- [Source: architecture.md#API Boundaries] — Paradex Exchange: WebSocket (WSS) + REST
- [Source: architecture.md#Existing Technology Stack] — starknet-crypto
- [Source: architecture.md#Authentication Protocols] — SNIP-12 (Starknet) signing pour Paradex
- [Source: epics.md#Story 1.2] — Acceptance criteria originaux
- [Source: 1-1-connexion-websocket-vest.md] — Pattern de validation Story 1.1

## Definition of Done Checklist

- [ ] Code compiles sans warnings (`cargo build`)
- [ ] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [ ] Tests passent (`cargo test`)
- [ ] Connexion WebSocket établie avec credentials valides
- [ ] Log `[INFO] Paradex WebSocket connected` visible
- [ ] Authentification SNIP-12 fonctionnelle
- [ ] Credentials jamais en clair dans logs

## Dev Agent Record

### Agent Model Used

{{agent_model_name_version}}

### Change Log

<!-- Dev agent should add entries here as changes are made -->

### Completion Notes List

<!-- Dev agent should document learnings and notes here -->

### File List

<!-- Dev agent should track all modified/created files here -->
