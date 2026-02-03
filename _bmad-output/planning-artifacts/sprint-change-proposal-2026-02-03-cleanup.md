# Sprint Change Proposal: Core Module Cleanup

**Date**: 2026-02-03
**Trigger**: Story 7.3 "Remove Supabase from Critical Path"
**Scope**: Minor - Direct Implementation

---

## 1. Issue Summary

Pendant l'optimisation HFT (Epic 7), une analyse du module `src/core/` a révélé **~824 lignes de code mort** qui :
- Complexifient le debugging
- Augmentent la surface de bugs potentiels
- Référencent un ancien nom de crate (`y_bot`)

### Fichiers identifiés comme code mort

| Fichier | Lignes | Raison |
|---------|--------|--------|
| `reconnect.rs` | 178 | Non utilisé dans `main.rs` |
| `logging.rs` | 396 | Non utilisé dans `main.rs` |
| `SpreadMonitor` (spread.rs) | ~200 | Non utilisé dans `main.rs` |
| **Total** | **~824** | |

---

## 2. Impact Analysis

### Epic Impact
- **Epic 7** (Latence V1 HFT): ✅ Inchangé - Story 7.4 ajoutée
- **Epics futurs**: ✅ Aucun impact

### Artifact Conflicts
- **PRD**: ✅ Aucun conflit
- **Architecture**: ⚠️ Mise à jour nécessaire (retirer `logging.rs` de la doc)
- **Tests**: ⚠️ Suppression des tests associés

---

## 3. Recommended Approach

**Option 1: Ajustement Direct** ✅ Sélectionnée

| Critère | Évaluation |
|---------|------------|
| Effort | 🟢 **Low** (~2h) |
| Risque | 🟢 **Low** |
| Timeline Impact | Aucun |

---

## 4. Detailed Change Proposals

### Story 7.4: Core Module Cleanup (NEW)

**Epic**: 7 - Optimisation Latence V1 HFT

#### Task 1: Supprimer `reconnect.rs`
```diff
- pub mod reconnect;
```
- Supprimer `src/core/reconnect.rs`
- Retirer du `src/core/mod.rs`

#### Task 2: Supprimer `logging.rs`
```diff
- pub mod logging;
- pub use logging::{init_logging, SanitizedValue, ...};
```
- Supprimer `src/core/logging.rs`
- Retirer du `src/core/mod.rs`

#### Task 3: Supprimer `SpreadMonitor` de `spread.rs`
```diff
- pub struct SpreadMonitor { ... }
- impl SpreadMonitor { ... }
- pub enum SpreadMonitorError { ... }
```
- Garder `SpreadCalculator` (utilisé)
- Supprimer `SpreadMonitor` et ses tests
- Mettre à jour exports dans `mod.rs`

#### Task 4: Validation
- `cargo build --release`
- `cargo test --lib`
- `cargo clippy`

---

## 5. Implementation Handoff

**Scope**: Minor - Direct Implementation

**Routed to**: Development Team (Dev Agent)

**Deliverables**:
1. Story 7.4 créée dans `epics.md`
2. Sprint status mis à jour
3. Code cleanup implémenté

**Success Criteria**:
- [ ] `reconnect.rs` supprimé
- [ ] `logging.rs` supprimé
- [ ] `SpreadMonitor` supprimé de `spread.rs`
- [ ] Build ✅
- [ ] Tests ✅
- [ ] Clippy ✅
