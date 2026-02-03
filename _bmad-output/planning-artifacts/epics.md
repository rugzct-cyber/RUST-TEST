---
stepsCompleted: ['step-01-validate-prerequisites', 'step-02-design-epics', 'step-03-create-stories', 'step-04-final-validation']
inputDocuments:
  - prd.md
  - architecture.md
workflowType: 'epics-and-stories'
project_name: 'bot4'
workflowStatus: 'complete'
totalEpics: 8
totalStories: 32
frsCovered: 21
---

# bot4 - Epic Breakdown

## Overview

This document provides the complete epic and story breakdown for bot4, decomposing the requirements from the PRD, UX Design if it exists, and Architecture requirements into implementable stories.

## Requirements Inventory

### Functional Requirements

**Market Data (FR1-4):**
- **FR1:** Le système peut se connecter simultanément aux WebSockets de Vest et Paradex
- **FR2:** Le système peut recevoir et parser les orderbooks en temps réel
- **FR3:** Le système peut calculer le spread entry/exit entre les deux exchanges
- **FR4:** Le système peut détecter quand un spread dépasse le seuil configuré

**Execution (FR5-9):**
- **FR5:** Le système peut placer un ordre long sur un exchange
- **FR6:** Le système peut placer un ordre short sur un exchange
- **FR7:** Le système peut exécuter les deux ordres simultanément (delta-neutral)
- **FR8:** Le système peut retenter un ordre échoué (retry logic)
- **FR9:** Le système peut fermer automatiquement l'autre leg si les retries échouent

**State Management (FR10-12):**
- **FR10:** Le système peut sauvegarder les positions ouvertes dans Supabase
- **FR11:** Le système peut restaurer l'état des positions après un redémarrage
- **FR12:** Le système peut maintenir un état in-memory cohérent

**Configuration (FR13-15):**
- **FR13:** L'opérateur peut configurer les paires de trading via YAML
- **FR14:** L'opérateur peut configurer les seuils de spread via YAML
- **FR15:** L'opérateur peut configurer les credentials via `.env`

**Resilience (FR16-18):**
- **FR16:** Le système peut se reconnecter automatiquement après un disconnect WebSocket
- **FR17:** Le système peut s'arrêter proprement sur SIGINT
- **FR18:** Le système ne laisse pas d'ordres orphelins après shutdown

**Observability (FR19-21):**
- **FR19:** Le système peut émettre des logs JSON structurés
- **FR20:** Le système peut redacter automatiquement les credentials dans les logs
- **FR21:** Le système peut logger chaque événement de trading avec contexte

### NonFunctional Requirements

**Performance:**
- **NFR1:** Calcul de spread < 2ms (HFT critical path)
- **NFR2:** Detection-to-order latency < 500ms (opportunité expire vite)
- **NFR3:** OrderBook parsing < 1ms (pas de bottleneck data)

**Security:**
- **NFR4:** Private keys jamais en clair dans les logs
- **NFR5:** Credentials storage via `.env` hors du repo git
- **NFR6:** Network security WSS (TLS) uniquement
- **NFR7:** No exposure — Auto-close on failed leg

**Reliability:**
- **NFR8:** Uptime > 99% pendant heures de trading
- **NFR9:** Reconnexion auto < 5s après disconnect
- **NFR10:** State recovery — Positions restaurées après restart
- **NFR11:** Graceful shutdown — No orphan orders

**Integration:**
- **NFR12:** Vest API compatible avec version actuelle
- **NFR13:** Paradex API compatible avec version actuelle
- **NFR14:** Supabase connexion stable pour state persistence

### Additional Requirements

**From Architecture — Brownfield Context:**
- Projet brownfield avec ~8,900 lignes de code Rust existant
- Structure modulaire existante (adapters/, core/, config/) à conserver
- Code core fonctionnel et testé (SpreadCalculator, VWAP, ExchangeAdapter)

**From Architecture — Phase 0 Cleanup:**
- Pattern "scout" à supprimer — inexistant dans MVP
- Intentions v3 à supprimer — non implémentées
- Code mort à identifier et supprimer
- Flow à simplifier vers exécution directe

**From Architecture — Implementation Priorities:**
- Créer `src/core/state.rs` pour Supabase position sync (High priority)
- Créer `src/core/execution.rs` pour order execution logic (High priority)
- Multi-task pipeline setup dans `runtime.rs`
- Retry + auto-close logic

**From Architecture — Authentication Protocols:**
- EIP-712 signing pour Vest
- SNIP-12 (Starknet) signing pour Paradex
- Isolated dans adapter modules

**From Architecture — Testing Requirements:**
- Exécuter `cargo clippy` avant commit
- Suivre `rustfmt` formatting
- Utiliser patterns d'erreur `thiserror`
- Logger avec `tracing` macros

### FR Coverage Map

| FR | Epic | Description |
|----|------|-------------|
| FR1 | Epic 1 | Connexion simultanée WebSockets Vest + Paradex |
| FR2 | Epic 1 | Réception et parsing orderbooks temps réel |
| FR3 | Epic 1 | Calcul spread entry/exit |
| FR4 | Epic 1 | Détection dépassement seuil |
| FR5 | Epic 2 | Placement ordre long |
| FR6 | Epic 2 | Placement ordre short |
| FR7 | Epic 2 | Exécution simultanée delta-neutral |
| FR8 | Epic 2 | Retry logic ordres échoués |
| FR9 | Epic 2 | Auto-close on failed leg |
| FR10 | Epic 3 | Sauvegarde positions Supabase |
| FR11 | Epic 3 | Restauration état après restart |
| FR12 | Epic 3 | Maintien état in-memory cohérent |
| FR13 | Epic 4 | Config paires via YAML |
| FR14 | Epic 4 | Config seuils spread via YAML |
| FR15 | Epic 4 | Config credentials via .env |
| FR16 | Epic 4 | Reconnexion auto WebSocket |
| FR17 | Epic 4 | Arrêt propre SIGINT |
| FR18 | Epic 4 | Pas d'ordres orphelins |
| FR19 | Epic 5 | Logs JSON structurés |
| FR20 | Epic 5 | Redaction credentials |
| FR21 | Epic 5 | Logging événements avec contexte |

## Epic List

## Epic 0: Cleanup & Foundation

Nettoyer le code legacy v3 et préparer une base solide pour le MVP.

**Outcome utilisateur :** Le codebase est propre, les patterns obsolètes supprimés, le bot compile et les tests passent.

**FRs couverts :** (Prérequis technique — pas de FR direct, mais requis par Architecture)

### Story 0.1: Suppression du Pattern Scout

As a développeur,
I want supprimer le pattern "scout" du codebase,
So that l'architecture soit simplifiée et alignée avec le MVP.

**Acceptance Criteria:**

**Given** le codebase actuel contient des références au pattern "scout"
**When** je recherche `scout` dans tout le projet
**Then** aucune référence au pattern scout n'existe
**And** le code compile sans erreurs (`cargo build`)
**And** tous les tests existants passent (`cargo test`)

### Story 0.2: Suppression du Code v3 Résiduel

As a développeur,
I want identifier et supprimer le code v3 non implémenté,
So that le codebase ne contienne que du code fonctionnel.

**Acceptance Criteria:**

**Given** le codebase contient des intentions v3 non implémentées
**When** j'exécute `cargo clippy --all-targets -- -D warnings`
**Then** aucun warning n'est généré
**And** les variables/fonctions inutilisées sont supprimées
**And** le code compile et les tests passent

### Story 0.3: Validation de la Structure Modulaire

As a développeur,
I want valider que la structure modulaire (adapters/, core/, config/) est cohérente,
So that l'implémentation du MVP puisse s'appuyer sur une base solide.

**Acceptance Criteria:**

**Given** la structure du projet avec adapters/, core/, config/
**When** j'inspecte les modules et leurs exports
**Then** chaque module a un `mod.rs` avec des exports explicites
**And** les dépendances inter-modules sont unidirectionnelles
**And** `cargo doc` génère une documentation sans erreurs

---

## Epic 1: Market Data Connection

L'opérateur peut streamer les orderbooks en temps réel depuis les deux exchanges.

**Outcome utilisateur :** Le bot se connecte aux WebSockets Vest + Paradex et affiche les spreads en temps réel dans les logs.

**FRs couverts :** FR1, FR2, FR3, FR4

### Story 1.1: Connexion WebSocket Vest

As a opérateur,
I want que le bot se connecte au WebSocket de Vest,
So that je puisse recevoir les données de marché en temps réel.

**Acceptance Criteria:**

**Given** les credentials Vest configurés dans `.env`
**When** le bot démarre
**Then** une connexion WebSocket WSS est établie avec Vest
**And** un log `[INFO] Vest WebSocket connected` est émis
**And** le bot gère l'authentification EIP-712

### Story 1.2: Connexion WebSocket Paradex

As a opérateur,
I want que le bot se connecte au WebSocket de Paradex,
So that je puisse recevoir les données de marché en temps réel.

**Acceptance Criteria:**

**Given** les credentials Paradex configurés dans `.env`
**When** le bot démarre
**Then** une connexion WebSocket WSS est établie avec Paradex
**And** un log `[INFO] Paradex WebSocket connected` est émis
**And** le bot gère l'authentification SNIP-12

### Story 1.3: Réception et Parsing des Orderbooks

As a opérateur,
I want que le bot reçoive et parse les orderbooks des deux exchanges,
So that les données soient prêtes pour le calcul de spread.

**Acceptance Criteria:**

**Given** les connexions WebSocket actives sur les deux exchanges
**When** un message orderbook est reçu
**Then** il est parsé en structure `Orderbook` standard
**And** les bids et asks sont correctement extraits
**And** un log `[DEBUG] Orderbook updated` est émis avec le pair
**And** le parsing s'exécute en < 1ms (NFR3)

### Story 1.4: Calcul de Spread Entry/Exit

As a opérateur,
I want que le bot calcule les spreads entry et exit,
So that je puisse voir les opportunités d'arbitrage.

**Acceptance Criteria:**

**Given** des orderbooks disponibles pour les deux exchanges
**When** les orderbooks sont mis à jour
**Then** le spread entry (Vest ask - Paradex bid) est calculé
**And** le spread exit (Paradex ask - Vest bid) est calculé
**And** le calcul s'exécute en < 2ms (NFR1)
**And** les spreads sont émis vers le channel correspondant

### Story 1.5: Détection de Dépassement de Seuil

As a opérateur,
I want que le bot détecte quand un spread dépasse le seuil configuré,
So that je sois alerté des opportunités de trade.

**Acceptance Criteria:**

**Given** un seuil de spread configuré dans `config.yaml` (ex: 0.30%)
**When** le spread calculé dépasse ce seuil
**Then** un log `[INFO] Spread opportunity detected: spread=X%, threshold=Y%` est émis
**And** un événement `SpreadOpportunity` est envoyé sur le channel d'exécution
**And** le seuil peut être entry ou exit selon configuration

---

## Epic 2: Delta-Neutral Execution

L'opérateur peut exécuter des trades delta-neutral avec protection automatique.

**Outcome utilisateur :** Le bot exécute simultanément long/short quand le spread dépasse le seuil, avec retry et auto-close si échec.

**FRs couverts :** FR5, FR6, FR7, FR8, FR9

### Story 2.1: Placement d'Ordre Long

As a opérateur,
I want que le bot puisse placer un ordre long sur un exchange,
So that je puisse ouvrir une position longue.

**Acceptance Criteria:**

**Given** une connexion active à un exchange
**When** un ordre long est demandé avec pair, size, et price
**Then** l'ordre est envoyé via l'API REST de l'exchange
**And** la réponse (succès ou échec) est retournée
**And** un log `[INFO] Order placed: side=long, pair=X, size=Y` est émis
**And** l'ordre est signé avec le protocole approprié (EIP-712/SNIP-12)

### Story 2.2: Placement d'Ordre Short

As a opérateur,
I want que le bot puisse placer un ordre short sur un exchange,
So that je puisse ouvrir une position courte.

**Acceptance Criteria:**

**Given** une connexion active à un exchange
**When** un ordre short est demandé avec pair, size, et price
**Then** l'ordre est envoyé via l'API REST de l'exchange
**And** la réponse (succès ou échec) est retournée
**And** un log `[INFO] Order placed: side=short, pair=X, size=Y` est émis
**And** l'ordre est signé avec le protocole approprié (EIP-712/SNIP-12)

### Story 2.3: Exécution Delta-Neutral Simultanée

As a opérateur,
I want que le bot exécute simultanément un ordre long et un ordre short,
So that ma position soit delta-neutral dès l'ouverture.

**Acceptance Criteria:**

**Given** une opportunité de spread détectée
**When** l'exécution delta-neutral est déclenchée
**Then** un ordre long est placé sur Exchange A
**And** un ordre short est placé sur Exchange B en parallèle
**And** les deux ordres sont envoyés dans une latence < 500ms (NFR2)
**And** un log `[TRADE] Entry executed: spread=X%, long=ExchA, short=ExchB` est émis

### Story 2.4: Retry Logic sur Échec d'Ordre

As a opérateur,
I want que le bot retente un ordre échoué,
So that les échecs temporaires ne bloquent pas l'exécution.

**Acceptance Criteria:**

**Given** un ordre qui échoue (timeout, rate limit, API error)
**When** l'échec est détecté
**Then** l'ordre est retenté jusqu'à 3 fois (configurable)
**And** un délai fixe est appliqué entre les retries
**And** un log `[RETRY] Order failed, attempt 2/3...` est émis à chaque retry
**And** si tous les retries échouent, un événement d'échec est propagé

### Story 2.5: Auto-Close sur Échec de Leg

As a opérateur,
I want que le bot ferme automatiquement la leg réussie si l'autre échoue,
So that je n'aie jamais de position directionnelle non couverte (NFR7).

**Acceptance Criteria:**

**Given** une exécution delta-neutral où une leg a réussi et l'autre a échoué après tous les retries
**When** l'échec définitif est confirmé
**Then** la leg réussie est automatiquement fermée (ordre opposé)
**And** un log `[SAFETY] Auto-closing successful leg to avoid exposure` est émis
**And** aucune position directionnelle non couverte ne reste ouverte
**And** l'état final est loggé avec le résumé de l'opération

---

## Epic 3: State Persistence

L'opérateur peut redémarrer le bot sans perdre l'état des positions.

**Outcome utilisateur :** Les positions ouvertes sont sauvegardées dans Supabase et restaurées après restart.

**FRs couverts :** FR10, FR11, FR12

### Story 3.1: Création du Module State Persistence

As a développeur,
I want créer le module `src/core/state.rs`,
So that la logique de persistence soit centralisée.

**Acceptance Criteria:**

**Given** le besoin de persister les positions
**When** le module `state.rs` est créé
**Then** il exporte les types `PositionState`, `StateManager`
**And** il est référencé dans `core/mod.rs`
**And** le code compile sans erreurs

### Story 3.2: Sauvegarde des Positions dans Supabase

As a opérateur,
I want que les positions ouvertes soient sauvegardées dans Supabase,
So that je ne perde pas l'état en cas de crash.

**Acceptance Criteria:**

**Given** une position delta-neutral ouverte avec succès
**When** la position est créée
**Then** elle est immédiatement sauvegardée dans Supabase (table `positions`)
**And** un log `[STATE] Position saved: pair=X, entry_spread=Y%` est émis
**And** les données incluent: pair, sizes, prices, timestamps, exchange ids
**And** la connexion Supabase est stable (NFR14)

### Story 3.3: Restauration de l'État après Redémarrage

As a opérateur,
I want que les positions soient restaurées après un redémarrage,
So that le bot reprenne son état précédent (NFR10).

**Acceptance Criteria:**

**Given** des positions ouvertes sauvegardées dans Supabase
**When** le bot démarre
**Then** les positions existantes sont chargées depuis Supabase
**And** l'état in-memory est initialisé avec ces positions
**And** un log `[STATE] Restored N positions from database` est émis
**And** le bot peut continuer à monitorer ces positions

### Story 3.4: Cohérence État In-Memory

As a opérateur,
I want que l'état in-memory reste cohérent avec Supabase,
So that les décisions du bot soient basées sur des données fiables.

**Acceptance Criteria:**

**Given** des positions en mémoire et dans Supabase
**When** une position est mise à jour (close, partial fill)
**Then** l'état in-memory est mis à jour immédiatement
**And** Supabase est synchronisé de manière asynchrone
**And** en cas d'échec de sync, un retry est effectué
**And** un warning est loggé si la sync échoue après retries

---

## Epic 4: Configuration & Operations

L'opérateur peut configurer et opérer le bot de manière sécurisée.

**Outcome utilisateur :** Configuration via YAML/env, arrêt propre sur Ctrl+C, pas d'ordres orphelins.

**FRs couverts :** FR13, FR14, FR15, FR16, FR17, FR18

### Story 4.1: Configuration des Paires via YAML

As a opérateur,
I want configurer les paires de trading via `config.yaml`,
So that je puisse changer les paires sans modifier le code.

**Acceptance Criteria:**

**Given** un fichier `config.yaml` avec une section `pairs`
**When** le bot démarre
**Then** les paires configurées sont chargées
**And** le bot s'abonne aux orderbooks de ces paires
**And** un log `[CONFIG] Loaded pairs: [ETH-USD, BTC-USD, ...]` est émis
**And** une erreur claire est loggée si le format est invalide

### Story 4.2: Configuration des Seuils de Spread

As a opérateur,
I want configurer les seuils de spread via `config.yaml`,
So that je puisse ajuster la sensibilité du bot.

**Acceptance Criteria:**

**Given** un fichier `config.yaml` avec `entry_threshold` et `exit_threshold`
**When** le bot démarre
**Then** les seuils sont validés (> 0, < 100%)
**And** ils sont utilisés pour la détection de spreads
**And** un log `[CONFIG] Thresholds: entry=X%, exit=Y%` est émis
**And** une erreur est levée si les seuils sont invalides

### Story 4.3: Configuration des Credentials via .env

As a opérateur,
I want configurer les credentials via `.env`,
So that mes clés privées ne soient jamais dans le code (NFR4, NFR5).

**Acceptance Criteria:**

**Given** un fichier `.env` avec les credentials des exchanges
**When** le bot démarre
**Then** les credentials sont chargés depuis `.env`
**And** ils ne sont jamais loggés en clair
**And** `SanitizedValue` est utilisé pour tout affichage
**And** une erreur claire est levée si un credential manque

### Story 4.4: Reconnexion Automatique WebSocket

As a opérateur,
I want que le bot se reconnecte automatiquement après un disconnect,
So that le trading continue sans intervention manuelle (NFR9).

**Acceptance Criteria:**

**Given** une connexion WebSocket active
**When** la connexion est perdue (timeout, network error)
**Then** le bot tente de se reconnecter automatiquement
**And** un backoff exponentiel est appliqué (max 5s)
**And** un log `[RECONNECT] Attempting reconnection to X...` est émis
**And** la reconnexion est établie en < 5s (NFR9)

### Story 4.5: Arrêt Propre sur SIGINT

As a opérateur,
I want que le bot s'arrête proprement sur Ctrl+C,
So that aucune ressource ne soit laissée pendante (NFR11).

**Acceptance Criteria:**

**Given** le bot en cours d'exécution
**When** je presse Ctrl+C (SIGINT)
**Then** un signal de shutdown est broadcasté à toutes les tâches
**And** les connexions WebSocket sont fermées proprement
**And** un log `[SHUTDOWN] Graceful shutdown initiated` est émis
**And** le process se termine avec exit code 0

### Story 4.6: Protection contre les Ordres Orphelins

As a opérateur,
I want qu'aucun ordre ne reste orphelin après shutdown,
So that je n'aie pas de positions imprévues.

**Acceptance Criteria:**

**Given** des ordres en attente lors du shutdown
**When** le shutdown est déclenché
**Then** les ordres pending sont annulés via l'API
**And** un log `[SHUTDOWN] Cancelled N pending orders` est émis
**And** le bot ne quitte qu'après confirmation d'annulation
**And** un log final `[SHUTDOWN] Clean exit, no pending orders` est émis

---

## Epic 5: Observability & Logging

L'opérateur peut comprendre et debugger le comportement du bot.

**Outcome utilisateur :** Logs JSON structurés avec credentials redactés, chaque événement logué avec contexte.

**FRs couverts :** FR19, FR20, FR21

### Story 5.1: Logs JSON Structurés

As a opérateur,
I want que le bot émette des logs JSON structurés,
So that je puisse les parser et les analyser facilement.

**Acceptance Criteria:**

**Given** le bot en cours d'exécution
**When** un événement est loggé
**Then** le log est émis au format JSON
**And** chaque log contient: timestamp, level, message, fields contextuels
**And** les logs sont émis sur stdout
**And** le format est compatible avec des outils comme `jq`

### Story 5.2: Redaction Automatique des Credentials

As a opérateur,
I want que les credentials soient automatiquement redactés dans les logs,
So that mes clés privées ne soient jamais exposées (NFR4).

**Acceptance Criteria:**

**Given** un log contenant des données sensibles (private keys, API secrets)
**When** le log est émis
**Then** les valeurs sensibles sont remplacées par `[REDACTED]`
**And** `SanitizedValue` wrapper est utilisé pour tous les credentials
**And** les clés privées, signatures, et tokens sont tous redactés
**And** même en mode debug, les credentials restent masqués

### Story 5.3: Logging des Événements de Trading avec Contexte

As a opérateur,
I want que chaque événement de trading soit loggé avec contexte complet,
So that je puisse tracer et debugger facilement.

**Acceptance Criteria:**

**Given** un événement de trading (spread détecté, ordre placé, position ouverte)
**When** l'événement se produit
**Then** un log structuré est émis avec:
- `pair`: la paire de trading
- `exchange`: l'exchange concerné
- `spread`: le spread calculé (si applicable)
- `timestamp`: horodatage précis
- `event_type`: type d'événement (SPREAD_DETECTED, ORDER_PLACED, etc.)
**And** les logs permettent de reconstruire la timeline des opérations

---

## Epic 6: Bot Automation & Integration

L'opérateur peut lancer le bot et celui-ci trade automatiquement.

**Outcome utilisateur :** `cargo run` → bot monitore spreads, exécute delta-neutral automatiquement, ferme positions automatiquement.

**FRs couverts :** Integration de FR1-FR21 (automatic execution loop)

### Story 6.1: Main Runtime Integration

As a opérateur,
I want que `main.rs` charge les credentials et démarre le runtime,
So that je puisse lancer le bot avec `cargo run`.

**Acceptance Criteria:**

**Given** `config.yaml` et `.env` sont configurés
**When** je lance `cargo run`
**Then** le bot:
- Charge credentials depuis `.env` (Story 4.3)
- Charge config depuis `config.yaml` (Stories 4.1, 4.2)
- Se connecte aux WebSockets Vest + Paradex (Stories 1.1, 1.2)
- S'abonne aux orderbooks (Story 1.3)
- Démarre la boucle de monitoring de spreads (Stories 1.4, 1.5)
**And** un log `[INFO] Bot runtime started` est émis
**And** le bot s'arrête proprement sur Ctrl+C (Story 4.5)

### Story 6.2: Automatic Delta-Neutral Execution

As a opérateur,
I want que les positions s'ouvrent automatiquement quand spread ≥ threshold,
So that je ne rate pas d'opportunités de trading.

**Acceptance Criteria:**

**Given** le monitoring de spreads est actif
**When** spread ≥ `spread_entry` threshold configuré
**Then** exécution delta-neutral est **automatiquement déclenchée**
**And** aucune intervention manuelle requise
**And** la logique de Story 2.3 (simultaneous long/short) est utilisée
**And** en cas de succès, position persistée dans Supabase (Story 3.2)
**And** en cas d'échec d'un leg, auto-close logic déclenchée (Story 2.5)
**And** un log `[TRADE] Auto-executed: spread=X%` est émis

### Story 6.3: Automatic Position Monitoring & Exit

As a opérateur,
I want que les positions se ferment automatiquement quand spread ≤ exit threshold,
So que je capture les profits sans monitoring manuel.

**Acceptance Criteria:**

**Given** une position delta-neutral ouverte
**When** spread ≤ `spread_exit` threshold configuré
**Then** position est **automatiquement fermée**
**And** les deux legs sont fermés simultanément
**And** Supabase est mis à jour (Story 3.4)
**And** un log `[TRADE] Auto-closed: spread=X%` est émis

**Given** des positions restaurées de Supabase au démarrage (Story 3.3)
**When** le bot reprend le monitoring
**Then** il tracke les conditions de sortie pour ces positions

### Story 6.5: End-to-End Integration Test

As a QA engineer,
I want un test automatisé du cycle complet,
So que je puisse vérifier que tout fonctionne end-to-end.

**Acceptance Criteria:**

**Given** un test d'intégration `tests/integration/full_cycle.rs`
**When** le test est exécuté avec `cargo test --test full_cycle`
**Then** le test couvre:
- Chargement config + credentials
- Connexion aux exchanges (testnet ou mock)
- Détection spread (mock spread opportunity)
- Exécution delta-neutral
- Persistence Supabase
- Fermeture automatique position
- Vérification état final
**And** le test passe sur CI/CD pipeline
**And** le test utilise testnet ou mocked exchanges

---

## Epic 7: Latency Optimization

L'opérateur bénéficie d'une exécution plus rapide pour capturer les opportunités HFT.

**Outcome utilisateur :** Latence d'exécution réduite de ~980ms à ~150-200ms via WebSocket orders et connection pooling.

**NFRs couverts :** NFR2 (Execution <500ms)

### Story 7.1: WebSocket Orders Paradex

As a opérateur,
I want que les ordres Paradex soient envoyés via WebSocket,
So that la latence d'exécution soit minimisée.

**Acceptance Criteria:**

**Given** une connexion WebSocket active avec Paradex
**When** un ordre est placé
**Then** il est envoyé via le WebSocket (pas REST)
**And** la latence d'ordre est < 150ms
**And** un log `[ORDER] Paradex WS order: latency=Xms` est émis
**And** les réponses d'ordre sont gérées de manière asynchrone

### Story 7.2: HTTP Connection Pooling

As a opérateur,
I want que les connexions HTTP soient réutilisées,
So que la latence REST (Vest) soit optimisée.

**Acceptance Criteria:**

**Given** le client HTTP `reqwest` utilisé pour Vest
**When** plusieurs requêtes REST sont envoyées
**Then** les connexions TCP/TLS sont réutilisées (keep-alive)
**And** la latence est réduite de ~50ms minimum
**And** les paramètres de pooling sont configurables
**And** un log au démarrage confirme la configuration du pool
