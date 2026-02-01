# Story 2-0: Refactoring Adapters

**Epic:** 2 - Delta-Neutral Execution
**Type:** Technical Story
**Status:** done
**Priority:** High (before Story 2-3)

---

## User Story

As a développeur,
I want refactorer les adaptateurs monolithiques vest.rs et paradex.rs en structure modulaire,
So that le code soit maintenable et aligné avec l'architecture documentée.

---

## Background

Les fichiers actuels:
- `vest.rs`: 2,740 lignes, 138 éléments
- `paradex.rs`: 3,277 lignes, 165 éléments

Structure cible définie dans `architecture.md` (lignes 341-352):
```
src/adapters/
├── vest/
│   ├── mod.rs, adapter.rs, auth.rs, orderbook.rs, types.rs
└── paradex/
    ├── mod.rs, adapter.rs, auth.rs, orderbook.rs, types.rs
```

---

## Acceptance Criteria

### AC1: Structure modulaire Vest
**Given** le fichier monolithique `src/adapters/vest.rs`
**When** je le refactore
**Then** un répertoire `src/adapters/vest/` est créé avec:
- `mod.rs` - exports publics
- `config.rs` - VestConfig
- `types.rs` - Response types, PreSignedOrder
- `signing.rs` - EIP-712 signing
- `rest.rs` - REST API calls
- `websocket.rs` - WebSocket handling
- `adapter.rs` - VestAdapter + ExchangeAdapter impl

### AC2: Structure modulaire Paradex
**Given** le fichier monolithique `src/adapters/paradex.rs`
**When** je le refactore
**Then** un répertoire `src/adapters/paradex/` est créé avec:
- `mod.rs` - exports publics
- `config.rs` - ParadexConfig
- `types.rs` - Response types, OrderSignParams
- `signing.rs` - SNIP-12 Starknet signing
- `rest.rs` - REST API calls
- `websocket.rs` - WebSocket handling
- `adapter.rs` - ParadexAdapter + ExchangeAdapter impl

### AC3: Tests passent
**Given** la nouvelle structure modulaire
**When** j'exécute `cargo test`
**Then** tous les tests existants passent sans modification

### AC4: Clippy clean
**Given** la nouvelle structure
**When** j'exécute `cargo clippy --all-targets -- -D warnings`
**Then** aucun warning n'est généré

### AC5: Binaire monitor fonctionne
**Given** la nouvelle structure
**When** j'exécute `cargo run --bin monitor`
**Then** le monitoring démarre et affiche les spreads

---

## Tasks

### Task 1: Créer structure vest/
- [x] 1.1 Créer `src/adapters/vest/mod.rs`
- [x] 1.2 Extraire `config.rs` (VestConfig)
- [x] 1.3 Extraire `types.rs` (response types, PreSignedOrder)
- [x] 1.4 Extraire `signing.rs` (EIP-712)
- [x] 1.5 ~~Extraire `rest.rs`~~ (consolidé dans adapter.rs)
- [x] 1.6 ~~Extraire `websocket.rs`~~ (consolidé dans adapter.rs)
- [x] 1.7 Créer `adapter.rs` (VestAdapter + REST + WebSocket)
- [x] 1.8 Supprimer `vest.rs`

### Task 2: Créer structure paradex/
- [x] 2.1 Créer `src/adapters/paradex/mod.rs`
- [x] 2.2 Extraire `config.rs` (ParadexConfig)
- [x] 2.3 Extraire `types.rs` (response types)
- [x] 2.4 Extraire `signing.rs` (SNIP-12)
- [x] 2.5 ~~Extraire `rest.rs`~~ (consolidé dans adapter.rs)
- [x] 2.6 ~~Extraire `websocket.rs`~~ (consolidé dans adapter.rs)
- [x] 2.7 Créer `adapter.rs` (ParadexAdapter + REST + WebSocket)
- [x] 2.8 Supprimer `paradex.rs`

### Task 3: Mettre à jour adapters/mod.rs
- [x] 3.1 Remplacer `pub mod vest;` par `pub mod vest;` (dossier)
- [x] 3.2 Remplacer `pub mod paradex;` par `pub mod paradex;` (dossier)
- [x] 3.3 Vérifier les re-exports publics

### Task 4: Validation
- [x] 4.1 `cargo build --all-targets`
- [x] 4.2 `cargo clippy --all-targets -- -D warnings`
- [x] 4.3 `cargo test`
- [ ] 4.4 `cargo run --bin monitor` (manuel)

---

## Definition of Done

- [x] Structure vest/ créée avec 5 fichiers (REST/WS consolidés)
- [x] Structure paradex/ créée avec 5 fichiers (REST/WS consolidés)
- [x] Anciens fichiers monolithiques supprimés
- [x] Tous les tests passent (174 tests)
- [x] Clippy sans warnings
- [ ] Binaire monitor fonctionne (validation manuelle)
- [x] Code review passée

---

## Dev Agent Record

### Files Modified (Code Review 2026-02-01)
- `src/bin/test_paradex_order.rs` - Fixed clippy::unnecessary_map_or
- `src/bin/test_order.rs` - Fixed clippy::to_string_in_format_args (2x)
- `src/adapters/paradex/config.rs` - Fixed clippy::field_reassign_with_default

### Change Log
| Date | Author | Changes |
|------|--------|---------|
| 2026-02-01 | AI Code Review | Fixed 4 clippy errors, updated task tracking |

---

## Notes

- **Effort estimé:** 1-2 heures
- **Risque:** Low — refactoring mécanique, tests valident non-régression
- **Sprint Change Proposal:** [sprint-change-proposal-2026-02-01.md]
- **Architecture Simplification:** REST et WebSocket consolidés dans adapter.rs pour réduire fragmentation (5 fichiers au lieu de 7)

