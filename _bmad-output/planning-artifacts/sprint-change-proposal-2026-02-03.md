# Sprint Change Proposal - Latency Optimization

**Date:** 2026-02-03  
**Auteur:** Workflow Correct-Course  
**Statut:** ✅ APPROUVÉ

---

## 1. Résumé du Problème

### Déclencheur
Lors du premier test live complet du bot (Epic 6 terminé), une latence d'exécution de **979ms** a été mesurée sur Paradex.

### Évidence
```
2026-02-03T15:38:20.080308Z  INFO hft_bot::adapters::paradex::adapter: 
📊 Order latency breakdown: signature=0ms, json=3μs, http=978ms, parse=8μs, total=978ms
```

### Impact
- **NFR Performance violé:** Target `Execution <500ms` → Actuel: 979ms
- Opportunités de trading perdues pendant la latence
- Compétitivité réduite vs autres bots HFT

---

## 2. Analyse d'Impact

### Epic Impact
| Epic | Statut | Impact |
|------|--------|--------|
| Epic 1-4 | Done | Aucun |
| Epic 5 | Backlog | Aucun |
| Epic 6 | Done | Fonctionne, mais performance suboptimale |
| **Epic 7** | **Nouveau** | À créer pour l'optimisation |

### Artifact Impact
| Artifact | Modification requise |
|----------|---------------------|
| `architecture.md` | Mise à jour section API Boundaries |
| `epics.md` | Ajout Epic 7 |
| `sprint-status.yaml` | Ajout Epic 7 et stories |

---

## 3. Solution Proposée

### Approche: Optimisation Hybride (2 volets)

#### Volet 1: WebSocket Orders pour Paradex
- **Quoi:** Envoyer les ordres via connexion WebSocket existante au lieu de REST
- **Pourquoi:** Connexion déjà établie = pas de handshake TCP/TLS
- **Gain:** ~800ms → ~100ms (≈700ms économisés)
- **Fichiers:** `adapters/paradex/adapter.rs`, `adapters/paradex/ws.rs`

#### Volet 2: HTTP Connection Pooling
- **Quoi:** Configurer `reqwest` pour réutiliser les connexions HTTP
- **Pourquoi:** Éviter le handshake TLS à chaque requête REST (Vest)
- **Gain:** ~150ms par requête
- **Fichiers:** `adapters/vest/adapter.rs`, configuration HTTP client

### Résultat Attendu
| Métrique | Avant | Après |
|----------|-------|-------|
| Latence Paradex | 978ms | ~100ms |
| Latence Vest | ~200ms | ~100ms |
| **Latence totale** | **~980ms** | **~150-200ms** |

---

## 4. Plan d'Implémentation

### Nouvel Epic 7: Latency Optimization

#### Story 7.1: WebSocket Orders Paradex
**Description:** Implémenter l'envoi d'ordres via WebSocket sur Paradex

**Tâches:**
1. Rechercher la documentation Paradex WS pour les ordres
2. Implémenter `place_order_ws()` dans `ParadexAdapter`
3. Ajouter gestion des réponses async via WS
4. Mettre à jour `execute_delta_neutral` pour utiliser WS sur Paradex
5. Tests unitaires et validation live

**Critères d'acceptation:**
- [ ] Ordres envoyés via WebSocket
- [ ] Latence < 150ms mesurée
- [ ] Logs avec breakdown de latence WS

---

#### Story 7.2: HTTP Connection Pooling
**Description:** Optimiser les connexions HTTP pour Vest avec connection pooling

**Tâches:**
1. Vérifier configuration actuelle du client `reqwest`
2. Configurer `pool_idle_timeout` et `pool_max_idle_per_host`
3. S'assurer que les connexions sont réutilisées (keep-alive)
4. Mesurer amélioration de latence
5. Tests de validation

**Critères d'acceptation:**
- [ ] Client HTTP configuré avec pooling
- [ ] Logs confirmant réutilisation des connexions
- [ ] Latence Vest réduite de ~50ms minimum

---

## 5. Effort et Timeline

| Story | Effort | Risque |
|-------|--------|--------|
| 7.1 WS Orders | 2-3 jours | Medium (nouvelle API) |
| 7.2 HTTP Pooling | 0.5 jour | Low (configuration) |
| **Total** | **3-4 jours** | **Medium** |

---

## 6. Handoff

### Classification: **Minor**
Changement technique qui peut être implémenté directement par l'équipe de développement.

### Responsabilités
| Rôle | Action |
|------|--------|
| **Dev** | Implémenter Stories 7.1 et 7.2 |
| **SM** | Créer les fichiers story et mettre à jour sprint-status.yaml |

### Critères de Succès
- [ ] Latence totale d'exécution < 250ms
- [ ] NFR Performance respecté (< 500ms)
- [ ] Tests live validés

---

## Approbation

- [ ] **Approuvé par:** ________________
- [ ] **Date:** ________________
