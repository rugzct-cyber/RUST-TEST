---
stepsCompleted: ['step-01-init', 'step-02-discovery', 'step-03-success', 'step-04-journeys', 'step-05-domain', 'step-06-innovation', 'step-07-project-type', 'step-08-scoping', 'step-09-functional', 'step-10-nonfunctional', 'step-11-polish', 'step-12-complete']
workflowStatus: complete
inputDocuments:
  - product-brief-bot4-2026-01-31.md
  - docs/index.md
  - docs/architecture.md
  - docs/data-models.md
  - docs/api-contracts.md
  - docs/source-tree.md
workflowType: 'prd'
documentCounts:
  briefs: 1
  research: 0
  projectDocs: 5
classification:
  projectType: api_backend
  domain: fintech
  complexity: high
  projectContext: brownfield
---

# Product Requirements Document - bot4

**Author:** rugz
**Date:** 2026-01-31

---

## Success Criteria

### User Success

- **Exécution automatique** : Le bot détecte et exécute des trades delta-neutral sans intervention manuelle
- **Latence acceptable** : <500ms de la détection à l'envoi d'ordre
- **Logs lisibles** : Événements clairs en JSON, pas de stacktraces cryptiques
- **Configuration simple** : Éditer `config.yaml` sans toucher au code

**Moment "Aha" MVP :** Le bot capture un spread qui a duré <1 seconde — impossible manuellement.

### Business Success

| Objectif | Timeline | Critère |
|----------|----------|---------|
| MVP fonctionnel | Immédiat | Bot connecté + détection + exécution |
| Rentabilité | Post-validation | Profitable sur période test |
| Sécurité | Phase 2 | Aucune liquidation single-leg |

### Technical Success

- Uptime >99% pendant les heures de trading actif
- Connexion WebSocket stable aux deux exchanges
- Calcul de spread <2ms (déjà vérifié en tests)

---

## Product Scope

### MVP — Minimum Viable Product

- ✅ Connexion WebSocket Vest + Paradex simultanée
- ✅ Détection spread temps réel
- ✅ Exécution delta-neutral (long + short)
- ✅ Config YAML (paires, seuils, capital)
- ✅ Logs structurés (tracing JSON)

### Phase 2 — Sécurité & Optimisation

- Protection anti-liquidation (monitoring ADL)
- VWAP+ avancé (slippage, depth)
- Précision d'entrée garantie

### Phase 3+ — Features

- Dashboard minimal (optionnel)
- Historique trades / DB
- Stratégies multiples

### Phase 4 — Production

- VPS / Hébergement
- Multi-paires parallèles
- Sécurité renforcée

---

## User Journeys

### 👤 Persona Principal : Rugz — Solo Trader / Vibecoder

**Profil :** Trader perps intermédiaire qui génère le code via IA. Stratégie delta-neutral entre DEX perps. Ne code pas manuellement — demande à l'IA pour tout debugging.

---

### Journey 1 : Happy Path — Trading Automatisé

**Opening Scene :** Rugz ouvre son terminal, vérifie que ses clés API sont configurées dans `.env`, et lance le bot avec `cargo run --release`.

**Rising Action :**
1. Le bot se connecte aux WebSockets Vest et Paradex
2. Il commence à streamer les orderbooks et calculer les spreads
3. Les logs JSON affichent les spreads en temps réel
4. Un spread de 0.35% apparaît (seuil configuré : 0.30%)

**Climax :** Le bot détecte l'opportunité et exécute simultanément :
- Long sur Exchange A
- Short sur Exchange B
- Logs : `[TRADE] Entry executed: spread=0.35%, target=0.30%`

**Resolution :** Rugz voit le trade confirmé dans les logs. Position delta-neutral ouverte.

---

### Journey 2 : Edge Case — Retry sur Échec de Leg

**Opening Scene :** Le bot détecte un spread éligible et lance l'exécution.

**Rising Action :**
1. Ordre long exécuté avec succès sur Exchange A ✅
2. Ordre short sur Exchange B **échoue** (rate limit, timeout, slippage) ❌
3. **DANGER :** Position directionnelle non couverte !

**Climax :** Le système de retry entre en action :
- Retry immédiat de l'ordre short
- Si échec après N retries → **Annuler le long** (ou fermer la position)
- Logs clairs : `[RETRY] Short failed, attempt 2/3...`

**Resolution :** 
- Scénario OK : Retry réussit, position delta-neutral établie
- Scénario fallback : Retries épuisés, ordres annulés, pas de position directionnelle

---

### Journey 3 : Ops — Arrêt d'Urgence

**Opening Scene :** Rugz voit un comportement anormal dans les logs.

**Action :** 
- `Ctrl+C` dans le terminal
- Le bot capture le signal SIGINT
- Ferme proprement les connexions WebSocket
- Log final : `[SHUTDOWN] Clean exit, no pending orders`

**Resolution :** Le bot s'arrête sans laisser d'ordres orphelins.

---

### Journey 4 : Troubleshooting — Mode Vibecoder

**Opening Scene :** Un comportement inattendu se produit.

**Action :**
1. Rugz copie les logs problématiques
2. Il demande à l'IA : "Pourquoi le bot fait ça ?"
3. L'IA analyse les logs et propose un fix
4. Rugz applique le fix

**Requirements révélés :** Logs structurés JSON, contextuels, sans stacktraces cryptiques.

---

### Journey Requirements Summary

| Journey | Capabilities Révélées |
|---------|----------------------|
| Happy Path | WebSocket connect, spread calc, dual execution, structured logs |
| Retry Leg | Retry logic, rollback mechanism, clear error states |
| Arrêt Urgence | Graceful shutdown, SIGINT handling, no orphan orders |
| Troubleshooting | JSON logs, contextual info, human-readable events |

---

## Domain-Specific Requirements

### Sécurité (CRITIQUE)

| Risque | Mitigation |
|--------|------------|
| Clés privées exposées | `.env` hors du repo, `SanitizedValue` pour les logs |
| Fuite de credentials dans logs | Logs redactés automatiquement |
| Single-leg exposure | Retry logic + rollback |
| Connexion non sécurisée | WSS uniquement (TLS) |

### Contraintes Temps Réel

| Contrainte | Requirement |
|------------|-------------|
| Latence spread calc | <2ms |
| Latence execution | <500ms |
| Reconnexion auto | Sur disconnect WebSocket |
| Heartbeat | Ping/pong pour détecter connexion morte |

### Intégrations Exchange

| Exchange | Protocole | Auth |
|----------|-----------|------|
| Vest | WebSocket + REST | EIP-712 |
| Paradex | WebSocket + REST | Starknet SNIP-12 |

### Risques Spécifiques Crypto

| Risque | Probabilité | Impact | Mitigation |
|--------|-------------|--------|------------|
| Exchange down | Medium | Trade raté | Detect + log, no action |
| Rate limiting | Medium | Retry delay | Exponential backoff |
| API change | Low | Bot cassé | Versionner les adapters |
| Slippage excessif | Medium | P&L réduit | VWAP (Phase 2) |

---

## Backend Specific Requirements

### Architecture Runtime

| Composant | Implémentation MVP |
|-----------|-------------------|
| Config loading | YAML (`config.yaml`) + `.env` pour secrets |
| Communication | Tokio channels (`broadcast` pour shutdown) |
| State | In-memory avec persistence des positions |
| Logging | tracing JSON, credentials redactés |

### Comportement Runtime

| Behavior | MVP Requirement |
|----------|-----------------|
| Reconnexion auto | ✅ Sur disconnect WebSocket |
| State persistence | ✅ Positions ouvertes sauvegardées |
| Multi-paires | ❌ Une seule paire à la fois |
| Graceful shutdown | ✅ SIGINT handling, no orphan orders |

### Code Cleanup Required

> ⚠️ **IMPORTANT**: Le codebase actuel contient des résidus de bot3/v3 qui doivent être nettoyés :
> - Pattern "scout" inexistant dans le MVP
> - Supprimer les intentions de v3 non implémentées
> - Simplifier l'architecture vers un flow direct

### Skip Sections (non pertinentes pour ce type)

- ❌ API publique / endpoints REST
- ❌ SDK / clients
- ❌ Versioning API
- ❌ Rate limiting côté serveur

---

## Project Scoping & Phased Development

### MVP Strategy

**Approche :** Problem-Solving MVP — Bot fonctionnel minimal qui résout le problème d'arbitrage

**Séquence de développement :**
1. **Phase 0 (Cleanup)** — Nettoyer le code v3 résiduel
2. **Phase 1 (MVP)** — Exécution delta-neutral fonctionnelle
3. **Phase 2 (Security)** — Protection anti-liquidation
4. **Phase 3+ (Features)** — Dashboard, stratégies

### MVP Feature Set (Phase 1)

**Must-Have :**
- WebSocket dual connect (Vest + Paradex)
- Spread detection temps réel
- Dual execution (long + short simultané)
- Retry logic avec auto-close on failure
- Config YAML
- Logs JSON structurés
- State persistence via Supabase (positions ouvertes)
- Reconnexion auto WebSocket
- Graceful shutdown

**Hors MVP :**
- Protection anti-liquidation (Phase 2)
- VWAP avancé (Phase 2)
- Multi-paires (Phase 4)
- Dashboard (Phase 3)

### Risk Mitigation Strategy

| Risk Type | Approach |
|-----------|----------|
| Technical | Cleanup v3 code first → évite confusion |
| Market | Single pair pour valider → scale après |
| Execution | Auto-close on failed leg → pas d'exposure |

### Dépendance Identifiée

> 📦 **Supabase** : Base de données existante de v3 réutilisée pour persister l'état des positions

---

## Functional Requirements

### Market Data

- **FR1:** Le système peut se connecter simultanément aux WebSockets de Vest et Paradex
- **FR2:** Le système peut recevoir et parser les orderbooks en temps réel
- **FR3:** Le système peut calculer le spread entry/exit entre les deux exchanges
- **FR4:** Le système peut détecter quand un spread dépasse le seuil configuré

### Execution

- **FR5:** Le système peut placer un ordre long sur un exchange
- **FR6:** Le système peut placer un ordre short sur un exchange
- **FR7:** Le système peut exécuter les deux ordres simultanément (delta-neutral)
- **FR8:** Le système peut retenter un ordre échoué (retry logic)
- **FR9:** Le système peut fermer automatiquement l'autre leg si les retries échouent

### State Management

- **FR10:** Le système peut sauvegarder les positions ouvertes dans Supabase
- **FR11:** Le système peut restaurer l'état des positions après un redémarrage
- **FR12:** Le système peut maintenir un état in-memory cohérent

### Configuration

- **FR13:** L'opérateur peut configurer les paires de trading via YAML
- **FR14:** L'opérateur peut configurer les seuils de spread via YAML
- **FR15:** L'opérateur peut configurer les credentials via `.env`

### Resilience

- **FR16:** Le système peut se reconnecter automatiquement après un disconnect WebSocket
- **FR17:** Le système peut s'arrêter proprement sur SIGINT
- **FR18:** Le système ne laisse pas d'ordres orphelins après shutdown

### Observability

- **FR19:** Le système peut émettre des logs JSON structurés
- **FR20:** Le système peut redacter automatiquement les credentials dans les logs
- **FR21:** Le système peut logger chaque événement de trading avec contexte

---

## Non-Functional Requirements

### Performance

| NFR | Target | Rationale |
|-----|--------|-----------|
| NFR1: Calcul de spread | <2ms | HFT critical path |
| NFR2: Detection-to-order latency | <500ms | Opportunité expire vite |
| NFR3: OrderBook parsing | <1ms | Pas de bottleneck data |

### Security

| NFR | Requirement |
|-----|-------------|
| NFR4: Private keys | Jamais en clair dans les logs |
| NFR5: Credentials storage | `.env` hors du repo git |
| NFR6: Network security | WSS (TLS) uniquement |
| NFR7: No exposure | Auto-close on failed leg |

### Reliability

| NFR | Target |
|-----|--------|
| NFR8: Uptime | >99% pendant heures de trading |
| NFR9: Reconnexion auto | <5s après disconnect |
| NFR10: State recovery | Positions restaurées après restart |
| NFR11: Graceful shutdown | No orphan orders |

### Integration

| NFR | Requirement |
|-----|-------------|
| NFR12: Vest API | Compatible avec version actuelle |
| NFR13: Paradex API | Compatible avec version actuelle |
| NFR14: Supabase | Connexion stable pour state persistence |

