# Sprint Change Proposal - Remove Supabase from Critical Path

**Date:** 2026-02-03  
**Auteur:** Workflow Correct-Course  
**Statut:** ✅ APPROUVÉ

---

## 1. Résumé du Problème

Query Supabase (~70ms) avant chaque trade + save après chaque trade ralentissent l'exécution.

**Objectif V1:** Bot HFT le plus rapide possible. Supabase = V2.

---

## 2. Solution Radicale

### Supprimer du chemin critique:
1. ❌ `load_positions()` avant chaque trade
2. ❌ `save_position()` après chaque trade  
3. ❌ `update_position()` après close
4. ❌ Vérification "position already exists"

### Conserver:
- ✅ Le module `state.rs` (pour V2)
- ✅ Les types `PositionState`, etc.
- ✅ `config.yaml` → `supabase.enabled: false`

---

## 3. Impact

### Files à modifier

| Fichier | Modification |
|---------|--------------|
| `src/core/runtime.rs` | Supprimer check `load_positions()` lignes 78-94 |
| `src/core/runtime.rs` | Supprimer `save_position()` après trade |
| `src/core/position_monitor.rs` | Supprimer `load_positions()` au démarrage task |
| `config.yaml` | `supabase.enabled: false` (déjà le cas) |

### Gain de latence
| Avant | Après | Gain |
|-------|-------|------|
| ~70ms (load) + ~76ms (save) | 0ms | **~146ms** |

---

## 4. Trade-offs

| ✅ Avantages | ⚠️ Inconvénients |
|-------------|------------------|
| Latence minimale | Pas de persistence |
| Code simplifié | Si crash → positions perdues |
| Focus HFT pur | Pas de récupération après restart |

**Mitigation:** Position tracking manuel via logs + exchange dashboards.

---

## 5. Story 7.3: Remove Supabase from Critical Path

**As a** opérateur HFT,  
**I want** que le bot n'utilise pas Supabase pendant le trading,  
**So that** la latence soit minimale.

**Tasks:**
1. `runtime.rs`: Supprimer bloc lignes 76-95 (check positions)
2. `runtime.rs`: Supprimer appel `save_position()` après trade
3. `position_monitor.rs`: Désactiver ou simplifier
4. Valider avec test de latence

**Effort:** ~0.5 jour

---

## Approval

**[ ] Approuvé** - Supprimer Supabase du critical path  
**[ ] Rejeté**  
**[ ] Révision**
