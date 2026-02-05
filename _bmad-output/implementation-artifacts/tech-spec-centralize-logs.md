---
title: 'Centralisation des Logs'
slug: 'centralize-logs'
created: '2026-02-05'
status: 'complete'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, tracing, tracing-subscriber]
files_to_modify: [events.rs, execution.rs]
code_patterns: [TradingEvent, log_event, factory_methods]
test_patterns: [cargo_test, manual_terminal]
---

# Tech-Spec: Centralisation des Logs

**Created:** 2026-02-05

## Overview

### Problem Statement

Les logs sont dispersés dans 8+ fichiers avec des appels `info!()` directs. La Story 5.3 a créé `events.rs` pour 8 types d'événements trading (`TradingEvent`), mais ~50+ autres logs (`CONFIG`, `CONNECTION`, `ADAPTER_INIT`, `RUNTIME`, etc.) appellent directement `info!()` sans passer par le système centralisé.

**Conséquences :**
- Impossible de changer le format globalement
- Incohérence des champs structurés
- Difficile de filtrer/analyser avec `jq`

### Solution

Étendre `src/core/events.rs` pour couvrir TOUS les types d'événements du système, puis migrer tous les appels `info!()` directs vers `log_event()`.

### Scope

**In Scope:**
- Extension de `events.rs` avec nouvelles catégories (System, Connection, Adapter, etc.)
- Migration des appels `info!()` directs vers `log_event()`
- Fichiers cibles : `main.rs`, `runtime.rs`, `execution.rs`, `monitoring.rs`, `pyth.rs`
- Implémentation du format compact une ligne pour terminal live

**Out of Scope:**
- Binaires de test (`src/bin/*`)
- Story 5.2 (rédaction credentials)

---

## First Principles Analysis (Insights)

### Consommation des Logs
- **Terminal live** : l'opérateur regarde les logs en temps réel
- **Format compact** : toutes les données pertinentes sur une seule ligne
- **Élimination du bruit** : pas de DIR, SIZE, TH (connus via config)

### Abréviations Validées

| Abrév | Signification |
|-------|---------------|
| `LES` | Live Entry Spread (monitoring pré-entrée) |
| `LXS` | Live Exit Spread (monitoring post-entrée) |
| `ES` | Entry Spread (au fill) |
| `XS` | Exit Spread (au close) |
| `CAP` | Captured (profit total = ES + XS) |
| `PEL` | Prix Entrée Long |
| `PES` | Prix Entrée Short |
| `LAT` | Latency |
| `P` | Paradex |
| `V` | Vest |

### Format par Phase

| Tag | Phase | Données |
|-----|-------|---------|
| `[SCAN]` | Monitoring pré-entrée | `LES` |
| `[ENTRY]` | Fill exécuté | `ES`, `PEL:P=X`, `PES:V=Y`, `LAT` |
| `[HOLD]` | Monitoring post-entrée | `LXS` |
| `[EXIT]` | Clôture exécutée | `XS`, `CAP`, `LAT` |

### Exemples

```
[SCAN] LES=0.35%
[ENTRY] ES=0.34% PEL:P=90000 PES:V=90100 LAT=42ms
[HOLD] LXS=-0.08%
[EXIT] XS=-0.12% CAP=0.22% LAT=38ms
```

## Context for Development (Step 2 Investigation)

### Codebase Patterns

- **events.rs** : Système structuré existant avec `TradingEvent` enum, `log_event()` function
- **log_event()** : Prend un `&TradingEvent`, switch sur `event_type`, émet `info!()` ou `debug!()`
- **Factory methods** : `TradingEvent::spread_detected()`, `TradingEvent::trade_entry()`, etc.
- **Support JSON** : Story 5.1 a ajouté `LOG_FORMAT=json` via `tracing-subscriber`

### Files to Modify

| File | Logs à Migrer | Catégories |
| ---- | ------------- | ---------- |
| `src/main.rs` | 25 | CONFIG, CONNECTION, ADAPTER_INIT, RUNTIME, BOT_SHUTDOWN |
| `src/core/runtime.rs` | 6 | RUNTIME, POSITION_MONITORING |
| `src/core/monitoring.rs` | 3 | TASK_START, TASK_STOP |
| `src/core/execution.rs` | 13 | ADAPTER_RECONNECT, TRADE_*, POSITION_* |
| `src/core/pyth.rs` | 3 | PYTH_INIT, USDC_RATE |
| **TOTAL** | **~50** | |

### Files to Reference (Read-Only)

| File | Purpose |
| ---- | ------- |
| `src/core/events.rs` | Système existant à étendre |
| `src/config/logging.rs` | Init du subscriber (Story 5.1) |

### Technical Decisions

1. **Deux catégories d'événements** :
   - `TradingEvent` (existant) → phases SCAN/ENTRY/HOLD/EXIT
   - `SystemEvent` (nouveau) → CONFIG, CONNECTION, RUNTIME, etc.

2. **Format compact pour trading** : `[TAG] key=value key=value`
   - Pas de JSON verbeux dans le message, juste le tag et les données essentielles
   
3. **Logs système restent verbeux** : Moins fréquents, ok d'être descriptifs

4. **`log_event()` bifurque** selon le type pour choisir le format

---

## Implementation Plan

### Tasks

- [x] **T1**: Ajouter format compact à `log_event()` pour trading events
  - File: `src/core/events.rs`
  - Action: Modifier le match dans `log_event()` pour émettre `[TAG] key=value` au lieu du format verbeux
  - Notes: Tags = SCAN, ENTRY, HOLD, EXIT. Utiliser les abréviations validées (ES, XS, LES, LXS, PEL, PES, LAT, CAP)

- [x] **T2**: Ajouter `log_compact()` helper function
  - File: `src/core/events.rs`
  - Action: Créer une fonction `log_compact(tag: &str, fields: &[(&str, String)])` qui formate une ligne compacte
  - Notes: Permet de centraliser le formatage `[TAG] key=value key=value`

- [x] **T3**: Mettre à jour SPREAD_DETECTED pour format `[SCAN]`
  - File: `src/core/events.rs`
  - Action: Dans `log_event()`, le case `SpreadDetected` émet `[SCAN] LES=X%` 
  - Notes: Remplace le format verbeux actuel

- [x] **T4**: Mettre à jour TRADE_ENTRY pour format `[ENTRY]`
  - File: `src/core/events.rs`
  - Action: Le case `TradeEntry` émet `[ENTRY] ES=X% PEL:P=Y PES:V=Z LAT=Nms`
  - Notes: P et V dépendent de quelle exchange est long/short

- [x] **T5**: Mettre à jour POSITION_MONITORING pour format `[HOLD]`
  - File: `src/core/events.rs`
  - Action: Le case `PositionMonitoring` émet `[HOLD] LXS=X%`
  - Notes: Reste en DEBUG level, throttled

- [x] **T6**: Mettre à jour TRADE_EXIT pour format `[EXIT]`
  - File: `src/core/events.rs`
  - Action: Le case `TradeExit` émet `[EXIT] XS=X% CAP=Y% LAT=Nms`
  - Notes: CAP = entry_spread + exit_spread (profit total capturé)

- [x] **T7**: Nettoyer les logs dans `execution.rs` pour éviter doublons
  - File: `src/core/execution.rs`
  - Action: Supprimer les `info!()` directs pour TRADE_ENTRY/EXIT (déjà émis via TradingEvent)
  - Notes: Attention à ne pas casser les logs ADAPTER_RECONNECT, POSITION_DETAIL qui restent verbeux

- [x] **T8**: Vérification et tests
  - Action: `cargo build --release && cargo test`
  - Notes: Vérifier que les tests existants passent toujours

---

### Acceptance Criteria

- [x] **AC1**: Given le bot est démarré, when un spread dépasse le threshold, then le log affiche `[SCAN] LES=X.XX%` sur une seule ligne
- [x] **AC2**: Given une opportunité est exécutée, when les deux ordres sont remplis, then le log affiche `[ENTRY] ES=X% PEL:P=Y PES:V=Z LAT=Nms`
- [x] **AC3**: Given une position est ouverte, when le monitoring poll, then le log affiche `[HOLD] LXS=X%` (throttled, debug level)
- [x] **AC4**: Given une position est fermée, when les ordres de close sont exécutés, then le log affiche `[EXIT] XS=X% CAP=Y% LAT=Nms`
- [x] **AC5**: Given `LOG_FORMAT=json`, when le bot tourne, then les logs restent en JSON valide (pas de régression)
- [x] **AC6**: Given le code est compilé, when `cargo test` est lancé, then tous les tests passent (172 tests OK)

---

## Verification Plan

### Automated Tests

```bash
# Build check
cargo build --release

# Run all tests
cargo test

# Specific events.rs tests (if any fail, investigate)
cargo test --lib events
```

### Manual Verification

1. **Terminal live** : `cargo run` et observer les logs
2. **Vérifier format SCAN** : Attendre un poll avec spread > threshold → doit voir `[SCAN] LES=X%`
3. **Vérifier format ENTRY** : Si trade exécuté → doit voir `[ENTRY] ES=... PEL:P=... PES:V=... LAT=...`
4. **Vérifier JSON mode** : `LOG_FORMAT=json cargo run` → confirmer que le JSON est toujours valide

---

## Dependencies

- Aucune nouvelle dépendance Cargo
- Story 5.1 (LOG_FORMAT) doit être mergée ✅
- Story 5.3 (TradingEvent) doit être mergée ✅

---

## Notes

- Le format JSON reste accessible via `LOG_FORMAT=json`
- Les abréviations (LES, LXS, ES, XS, CAP, PEL, PES, LAT) sont pour le display compact uniquement
- Les logs système (CONFIG, CONNECTION, BOT_SHUTDOWN) restent en format verbeux - out of scope pour cette spec
- Focus sur les 4 phases trading critiques : SCAN, ENTRY, HOLD, EXIT


