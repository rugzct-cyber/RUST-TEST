---
stepsCompleted: [1, 2, 3, 4, 5, 6]
inputDocuments:
  - docs/index.md
  - docs/architecture.md
  - docs/data-models.md
  - docs/api-contracts.md
  - docs/source-tree.md
date: 2026-01-31
author: rugz
---

# Product Brief: bot4

## Executive Summary

Bot4 est un bot d'arbitrage HFT personnel conçu pour capturer automatiquement des opportunités de spread entre exchanges perpétuels (Vest/Paradex) en quelques millisecondes. Né de l'échec d'une version précédente trop complexe, ce projet adopte une approche MVP incrémentale : un core solide d'abord, des features ajoutées progressivement.

**Objectif principal :** Automatiser l'exécution delta-neutral sur des spreads impossibles à trader manuellement, avec une précision d'entrée garantie (pas d'entrée à 0.1% quand le seuil est 0.2%).

---

## Core Vision

### Problem Statement

Le trading d'arbitrage de spreads entre DEX perpétuels est impossible à exécuter manuellement :
- Les opportunités apparaissent et disparaissent en millisecondes
- La surveillance manuelle entraîne systématiquement des opportunités manquées
- Le slippage dû à la taille de position n'est pas pris en compte sans calcul VWAP
- L'exécution humaine ne peut garantir un prix optimal

### Problem Impact

- Opportunités de profit perdues quotidiennement sur des mouvements rapides de spread
- Fatigue de surveillance sans garantie de capture
- Exécution sous-optimale = marge réduite sur chaque trade
- Temps perdu vs. automation qui travaille 24/7

### Why Existing Solutions Fall Short

**Version précédente (bot3) :** Échec dû à une sur-ingénierie :
- Frontend, graphiques, base de données, analyse
- Stratégies multiples d'entrée/sortie
- Mutex, sécurité anti-liquidation
- **Résultat :** Code non-viable, trop complexe pour un premier projet

**Bots génériques :** Ne ciblent pas cette stratégie spécifique d'arbitrage delta-neutral sur perps DEX.

### Proposed Solution

**MVP bot4 :**
1. Système d'entrée delta-neutral automatique (long/short simultané)
2. Détection de spread en temps réel < 2ms
3. Exécution automatique quand spread ≥ seuil configuré
4. Précision d'entrée garantie (spread réel ≥ spread cible)

**Roadmap incrémentale :**
1. ✅ MVP : Auto-capture spreads delta-neutral
2. Protection anti-liquidation
3. VWAP+ (calcul slippage avancé)
4. Stratégies d'entrée complexes
5. Hébergement VPS (latence + sécurité)

### Key Differentiators

- **Stratégie unique :** Arbitrage delta-neutral sur perps DEX - pas mainstream
- **Proof of concept :** Hedge live sur le marché démontrant la viabilité
- **Architecture MVP :** Leçon apprise de bot3 → simplicité d'abord
- **Rust + Tokio :** Performance native < 2ms par calcul de spread

---

## Target Users

### Primary User: Rugz (Solo Trader)

**Profil :**
- Trader perps depuis +1 an, niveau intermédiaire
- **Vibecoder** : génère le code via IA, ne code pas manuellement
- Disponibilité : temps plein pour surveillance/monitoring
- Stratégie : arbitrage delta-neutral entre DEX perps

**Contexte actuel :**
- Surveillance manuelle des spreads Vest/Paradex
- Opportunités manquées sur mouvements rapides
- Hedge live en cours prouvant la viabilité de la stratégie

**Frustrations :**
- Vitesse humaine insuffisante vs. spreads en millisecondes
- Pas de calcul VWAP pour optimiser l'exécution
- Version précédente (bot3) trop complexe → échec

**Besoins clés :**
- Bot fiable qui exécute automatiquement
- Logs clairs et compréhensibles (pas de stacktraces)
- Config YAML simple sans toucher au code
- Précision d'entrée : spread réel ≥ spread cible

### Secondary Users

N/A - Projet personnel sans utilisateurs secondaires prévus.

### User Journey

1. **Configuration** → Éditer `config.yaml` (seuils spread, paires, capital)
2. **Lancement** → `cargo run --release`
3. **Monitoring** → Observer les logs en temps réel
4. **Moment Aha** → Le bot capture un trade impossible manuellement
5. **Validation** → Performance supérieure au trading manuel

---

## Success Metrics

### User Success Metrics

| Métrique | Target | Mesure |
|----------|--------|--------|
| **Précision d'entrée** | ±0.02% du spread cible | `spread_réel - spread_cible` |
| **Latence exécution** | < 500ms (optimal: < 100ms) | Time from opportunity detection to order sent |
| **Élimination friction manuelle** | 100% automatisé | Plus besoin de: entrer quantités, vérifier erreurs, revalider opportunité |
| **Capture d'opportunités** | > 80% des spreads éligibles | Spreads capturés / spreads détectés |

**Moment "Aha"** : Le bot exécute un trade delta-neutral complet (long + short) sur un spread qui dure < 1 seconde - impossible manuellement.

### Business Objectives

| Objectif | Timeline | Critère de succès |
|----------|----------|-------------------|
| **MVP fonctionnel** | 4 jours | Bot détecte spreads + exécute auto |
| **Rentabilité** | Post-MVP | Profitable sur période test |
| **Sécurité** | Phase 2 | Aucune liquidation single-leg |

### Key Performance Indicators

**KPIs Techniques :**
- Latence moyenne < 500ms
- Spread d'entrée ≥ spread_target - 0.02%
- Uptime > 99% pendant heures de trading

**KPIs Trading :**
- Taux de capture : opportunités exécutées / détectées
- Slippage réel vs. VWAP estimé
- P&L par trade (dépend de paramètres agressivité)

**KPIs Risque (Phase 2) :**
- 0 liquidations single-leg
- Distance ADL maintenue > seuil critique

---

## MVP Scope

### Core Features (4 jours)

| Feature | Description | Priorité |
|---------|-------------|----------|
| **Connexion WebSocket** | Vest + Paradex simultané, orderbook streaming | P0 |
| **Détection spread temps réel** | Calcul < 2ms, entry/exit spread dual | P0 |
| **Exécution delta-neutral** | Long + short simultané quand spread ≥ seuil | P0 |
| **Config YAML** | Paires, seuils spread, capital, leverage | P0 |
| **Logs structurés** | Tracing JSON, événements clairs | P0 |

### Out of Scope pour MVP

| Feature | Raison exclusion | Phase prévue |
|---------|------------------|--------------|
| Frontend / Dashboard | Complexité bot3, pas essentiel | Phase 3+ |
| Base de données / Historique | Pas critique pour trading | Phase 3+ |
| Protection anti-liquidation | Post-validation trading de base | Phase 2 |
| VWAP+ avancé | Optimisation post-MVP | Phase 2 |
| Stratégies multiples | Un seul mode d'abord | Phase 3+ |
| VPS / Hébergement | Dev local d'abord | Phase 4 |

### MVP Success Criteria

**Gate de validation (Go/No-Go Phase 2) :**
- ✅ Plusieurs jours de fonctionnement
- ✅ Trades exécutés avec succès
- ✅ Aucune erreur d'exécution
- ✅ Spread d'entrée respecte la tolérance ±0.02%
- ✅ Latence < 500ms confirmée

### Future Vision

**Phase 2 - Sécurité :**
- Protection anti-liquidation (surveillance ADL)
- VWAP+ pour optimisation slippage

**Phase 3 - Features :**
- Stratégies d'entrée avancées
- Dashboard minimal (optionnel)
- Historique trades

**Phase 4 - Production :**
- Hébergement VPS (latence + uptime)
- Sécurité renforcée
- Multi-paires parallèles
