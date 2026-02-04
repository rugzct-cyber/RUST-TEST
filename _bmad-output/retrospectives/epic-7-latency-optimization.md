# Rétrospective Epic 7: Latency Optimization

**Date:** 2026-02-04
**Epic Status:** ✅ Done (3/3 stories)

---

## 📊 Métriques Clés

| Métrique | Baseline | Résultat | Amélioration |
|----------|----------|----------|--------------|
| **Latence ordre Paradex** | 978ms | 442ms | **55%** ↓ |
| **Première requête** | +100-150ms | ~0ms | ✅ Warm-up |
| **Supabase latency** | ~70ms | 0ms | ✅ Supprimé |

---

## ✅ Ce Qui a Bien Marché

### 1. Pattern HTTP Pooling Réutilisable
- Créé pour Paradex (Story 7-1), répliqué pour Vest (Story 7-2)
- Config standard: `pool_max_idle_per_host=2`, `pool_idle_timeout=60s`, `tcp_keepalive=30s`
- **Leçon:** Les patterns devraient toujours être documentés pour réutilisation cross-adapter

### 2. Connection Warm-up
- Méthode `warm_up_http()` établit TCP/TLS au démarrage
- Élimine ~100-150ms latence sur première requête
- Appel `GET /system/time` (Paradex) ou `/account` (Vest)

### 3. V1 HFT Architecture (Story 7-3)
- Suppression complète de Supabase du chemin critique
- Lock-free monitoring avec `SharedOrderbooks` (Arc<RwLock>)
- Polling 40Hz sans blocage

---

## ⚠️ Challenges Rencontrés

### 1. Target <200ms Non Atteint
- **Analyse:** ~300-400ms incompressibles côté serveur Paradex (soumission StarkNet)
- **Leçon:** Toujours valider les limites physiques des APIs externes avant de définir des targets

### 2. Story 7-3 Hors-BMAD
- Changements majeurs de suppression Supabase faits en "vibecoding"
- **Leçon:** Même les changements urgents méritent une story formelle pour traçabilité

### 3. WebSocket Orders Paradex - Confusion Initiale
- **Discovery:** Paradex WebSocket = data subscriptions only, orders = REST API only
- **Leçon:** Valider capabilities API avant design de stories

---

## 🛠️ Travail Hors-BMAD (Vibecoding)

L'opérateur a effectué un nettoyage significatif en parallèle:

| Action | Impact |
|--------|--------|
| Suppression retry logic résiduel | ✅ Simplification code |
| Suppression auto-deleverage legacy | ✅ Moins de complexité |
| Correction logique calcul spread | ✅ Bot fonctionnel |
| Suppression dépendance Supabase | ✅ Latence réduite |
| Tentatives amélioration logs | ⚠️ Inconsistant, à reprendre |

---

## 🔴 Problèmes Identifiés (Nouveaux)

### Slippage Excessif
- **Target spread:** 0.10%
- **Spread exécuté:** ~0.02% (ou moins)
- **Gap:** ~80% entre détection et exécution
- **Action:** Créé Epic 8 avec Story 8.1 (Slippage Investigation)

### Logs Désorganisés
- Multiples tentatives de formatage inconsistantes
- Difficile d'analyser les actions du bot
- **Action:** Story 5.3 mise à jour avec approche "clean slate"

---

## 🎯 Actions Items

| # | Action | Propriétaire | Status |
|---|--------|--------------|--------|
| 1 | Documenter limite latence Paradex (~400ms server-side) | Dev | ✅ Dans story 7-1 |
| 2 | Créer Story 8.1 Slippage Investigation | SM | ✅ Ajouté à epics.md |
| 3 | Mettre à jour Story 5.3 avec clean slate | SM | ✅ Ajouté à epics.md |
| 4 | Investiguer gap détection → exécution | Dev | 🔜 Epic 8 |

---

## 🔮 Prochaines Étapes

### Priorité haute
1. **Epic 5 - Story 5.3:** Refonte logging de A à Z
2. **Epic 8 - Story 8.1:** Investigation slippage avec timing breakdown

### Backlog
- Epic 5 - Story 5.1: Logs JSON structurés
- Epic 5 - Story 5.2: Redaction credentials
- Epic 8 - Story 8.2: Speed optimization (après investigation)

---

## 📁 Fichiers Modifiés (Epic 7)

### Story 7.1 - WebSocket Orders Paradex
- `src/adapters/paradex/adapter.rs` - HTTP pooling, warm_up_http(), subscribe_orders()
- `src/bin/test_paradex_order.rs` - Test avec WebSocket order confirmations
- `src/main.rs` - Integration subscribe_orders()

### Story 7.2 - HTTP Connection Pooling
- `src/adapters/vest/adapter.rs` - HTTP pooling, warm_up_http()

### Story 7.3 - Remove Supabase (Hors-BMAD)
- Suppression modules Supabase du chemin critique
- Lock-free SharedOrderbooks implementation

---

## 💡 Insights pour Futurs Epics

1. **Patterns cross-adapter:** Documenter immédiatement pour réplication
2. **API Discovery:** Toujours lire la doc API complète avant story design
3. **Vibecoding:** Acceptable pour exploration, mais documenter les changements après coup
4. **Latence:** Distinguer optimisations client-side vs limites server-side
