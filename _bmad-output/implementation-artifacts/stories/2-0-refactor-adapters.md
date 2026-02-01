# Story 2-0: Refactoring Adapters

**Epic:** 2 - Delta-Neutral Execution
**Type:** Technical Story
**Status:** in-progress
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
- [ ] 1.1 Créer `src/adapters/vest/mod.rs`
- [ ] 1.2 Extraire `config.rs` (VestConfig)
- [ ] 1.3 Extraire `types.rs` (response types, PreSignedOrder)
- [ ] 1.4 Extraire `signing.rs` (EIP-712)
- [ ] 1.5 Extraire `rest.rs` (REST API calls)
- [ ] 1.6 Extraire `websocket.rs` (WebSocket handling)
- [ ] 1.7 Créer `adapter.rs` (VestAdapter)
- [ ] 1.8 Supprimer `vest.rs`

### Task 2: Créer structure paradex/
- [ ] 2.1 Créer `src/adapters/paradex/mod.rs`
- [ ] 2.2 Extraire `config.rs` (ParadexConfig)
- [ ] 2.3 Extraire `types.rs` (response types)
- [ ] 2.4 Extraire `signing.rs` (SNIP-12)
- [ ] 2.5 Extraire `rest.rs` (REST API calls)
- [ ] 2.6 Extraire `websocket.rs` (WebSocket handling)
- [ ] 2.7 Créer `adapter.rs` (ParadexAdapter)
- [ ] 2.8 Supprimer `paradex.rs`

### Task 3: Mettre à jour adapters/mod.rs
- [ ] 3.1 Remplacer `pub mod vest;` par `pub mod vest;` (dossier)
- [ ] 3.2 Remplacer `pub mod paradex;` par `pub mod paradex;` (dossier)
- [ ] 3.3 Vérifier les re-exports publics

### Task 4: Validation
- [ ] 4.1 `cargo build --all-targets`
- [ ] 4.2 `cargo clippy --all-targets -- -D warnings`
- [ ] 4.3 `cargo test`
- [ ] 4.4 `cargo run --bin monitor`

---

## Definition of Done

- [ ] Structure vest/ créée avec 7 fichiers
- [ ] Structure paradex/ créée avec 7 fichiers
- [ ] Anciens fichiers monolithiques supprimés
- [ ] Tous les tests passent
- [ ] Clippy sans warnings
- [ ] Binaire monitor fonctionne
- [ ] Code review passée

---

## Notes

- **Effort estimé:** 1-2 heures
- **Risque:** Low — refactoring mécanique, tests valident non-régression
- **Sprint Change Proposal:** [sprint-change-proposal-2026-02-01.md]
