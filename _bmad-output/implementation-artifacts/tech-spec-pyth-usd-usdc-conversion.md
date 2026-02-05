---
title: 'Intégration Pyth Network pour conversion USD/USDC'
slug: 'pyth-usd-usdc-conversion'
created: '2026-02-05'
status: 'ready-for-dev'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [rust, reqwest, tokio, pyth-hermes-api]
files_to_modify:
  - src/core/pyth.rs (NEW)
  - src/core/mod.rs
  - src/adapters/paradex/types.rs
  - src/adapters/paradex/adapter.rs
code_patterns: [atomic-cache, background-task, thread-safe-singleton]
test_patterns: [unit-tests-in-module, cargo-test]
---

# Tech-Spec: Intégration Pyth Network pour conversion USD/USDC

**Created:** 2026-02-05

## Overview

### Problem Statement

Paradex affiche et reçoit les prix en **USD** tandis que Vest et Lighter utilisent **USDC**. Cette différence de devise fausse le calcul du spread (~0.03% d'écart) et peut générer de faux signaux d'arbitrage.

### Solution

Intégrer l'API Pyth Hermes pour récupérer le taux USD/USDC, puis convertir les prix Paradex (USD → USDC) dans la méthode `to_orderbook()` avant qu'ils n'atteignent le SpreadCalculator.

### Scope

**In Scope:**
- Nouveau module `src/core/pyth.rs` pour gérer le taux USD/USDC
- Background task pour refresh du taux toutes les **15 minutes**
- Conversion des prix WS Paradex (orderbook) en USDC
- Fallback : garder le dernier taux connu si Pyth indisponible
- **Validation de sanité** : rejeter les taux hors `[0.90, 1.10]`
- **Retry au démarrage** : 3 tentatives avec backoff
- **Monitoring** : log WARN si rate change >0.5% entre refreshs

**Out of Scope:**
- Conversion des `average_entry_price` (déjà en USDC)
- Configuration dynamique de l'intervalle (hardcodé 15min)

## Context for Development

### Codebase Patterns

| Pattern | Localisation | Description |
|---------|-------------|-------------|
| Atomic cache | `adapters/types.rs` | `AtomicU64` pour compteurs thread-safe |
| Background task | `core/runtime.rs` | `tokio::spawn` avec channels |
| HTTP client | `adapters/paradex/adapter.rs` | `reqwest::Client` réutilisable |

### Files to Reference

| File | Purpose | Lignes clés |
|------|---------|-------------|
| `src/adapters/paradex/types.rs` | **Point d'intégration** : `to_orderbook()` | L117-163 |
| `src/adapters/paradex/adapter.rs` | Appels à `to_orderbook()` | L458, L481 |
| `src/core/spread.rs` | SpreadCalculator (non modifié) | - |

### Technical Decisions

1. **Option A** : Conversion dans `to_orderbook()`, transparent pour le reste du code
2. **Global singleton** : `UsdcRateCache` accessible via `Arc<AtomicU64>` passé aux adapters
3. **Fallback** : Si Pyth échoue, garde le dernier taux (init à 1.0)
4. **Refresh 15min** : Réactif aux variations du taux USDC
5. **Validation bounds** : Rejeter rates hors `[0.90, 1.10]` (protection valeurs aberrantes)
6. **Retry startup** : 3 tentatives avec exponential backoff avant fallback

## Implementation Plan

### Task 1: Créer le module Pyth (`src/core/pyth.rs`)

**Fichier:** `src/core/pyth.rs` [NEW]

```rust
// Structure:
pub struct UsdcRateCache { rate_micros: AtomicU64 }
pub async fn fetch_usdc_rate(client: &reqwest::Client) -> Result<f64>
pub fn spawn_rate_refresh_task(cache: Arc<UsdcRateCache>, client: reqwest::Client)
```

**Actions:**
- Créer struct `UsdcRateCache` avec `AtomicU64` (rate × 1_000_000)
- Implémenter `get_rate()` et `update(new_rate)` avec validation bounds `[0.90, 1.10]`
- Fonction `fetch_usdc_rate()` appelant Pyth Hermes API avec timeout 5s
- Fonction `fetch_with_retry()` : 3 tentatives, backoff 1s/2s/4s
- Log `WARN` si rate change >0.5% par rapport au précédent
- Background task avec `tokio::time::interval(Duration::from_secs(900))` (15min)

---

### Task 2: Exporter le module (`src/core/mod.rs`)

**Fichier:** `src/core/mod.rs` [MODIFY]

**Action:** Ajouter `pub mod pyth;`

---

### Task 3: Modifier `to_orderbook()` pour accepter le taux

**Fichier:** `src/adapters/paradex/types.rs` [MODIFY]

**Avant (L117):**
```rust
pub fn to_orderbook(&self) -> ExchangeResult<Orderbook>
```

**Après:**
```rust
pub fn to_orderbook(&self, usdc_rate: Option<f64>) -> ExchangeResult<Orderbook>
```

**Modification interne (L122-127):**
```rust
let mut price = level.price.parse::<f64>().map_err(...)?;
// Convertir USD → USDC si rate fourni
if let Some(rate) = usdc_rate {
    price = price / rate;  // Si USDC vaut 0.9997 USD, on divise
}
```

---

### Task 4: Passer le taux depuis l'adapter

**Fichier:** `src/adapters/paradex/adapter.rs` [MODIFY]

**Actions:**
1. Ajouter champ `usdc_rate_cache: Option<Arc<UsdcRateCache>>` à `ParadexAdapter`
2. Modifier appels `to_orderbook()` (L458, L481) pour passer `self.get_usdc_rate()`
3. Ajouter méthode helper `get_usdc_rate() -> Option<f64>`

---

### Task 5: Initialiser le cache au démarrage

**Fichier:** `src/main.rs` ou là où le bot est initialisé [MODIFY]

**Actions:**
1. Créer `Arc<UsdcRateCache>` au démarrage
2. Appeler `spawn_rate_refresh_task()`
3. Passer le cache à `ParadexAdapter::new()`

---

### Task 6: Tests unitaires

**Fichier:** `src/core/pyth.rs` [NEW - section tests]

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_rate_cache_default_is_one()
    #[test]
    fn test_rate_cache_update_and_get()
    #[test]
    fn test_rate_cache_rejects_out_of_bounds() // NEW: rates <0.90 or >1.10 rejected
    #[test]
    fn test_usd_to_usdc_conversion()
}
```

**Fichier:** `src/adapters/paradex/types.rs` [MODIFY - ajouter test]

```rust
#[test]
fn test_to_orderbook_with_usdc_conversion()
```

### Acceptance Criteria

**AC1: Taux récupéré au démarrage**
- Given: Le bot démarre
- When: La task de refresh Pyth s'exécute
- Then: Le taux USD/USDC est stocké dans le cache

**AC2: Prix convertis en USDC**
- Given: Un message orderbook Paradex avec prix 42000.00 USD
- When: `to_orderbook(Some(0.9997))` est appelé
- Then: Le prix dans l'Orderbook est 42012.60 USDC (42000 / 0.9997)

**AC3: Fallback fonctionne**
- Given: Pyth API indisponible
- When: Le refresh échoue
- Then: Le dernier taux connu est conservé, warning loggé

**AC4: Tests passent**
- Given: Tous les tests existants
- When: `cargo test` exécuté
- Then: Aucune régression, nouveaux tests passent

**AC5: Validation de sanité** (NEW)
- Given: Pyth retourne un rate de 0.50 (aberrant)
- When: `update()` est appelé
- Then: Le rate est rejeté, log `WARN`, ancien rate conservé

**AC6: Retry au démarrage** (NEW)
- Given: Pyth échoue les 2 premiers appels
- When: 3ème tentative réussit
- Then: Rate correctement initialisé, log `INFO` succès

**AC7: Monitoring changement de rate** (NEW)
- Given: Rate passe de 0.9997 à 0.9940 (>0.5% change)
- When: Refresh détecte le changement
- Then: Log `WARN` avec ancien et nouveau rate

## Verification Plan

### Automated Tests

```bash
# Exécuter tous les tests
cargo test

# Exécuter spécifiquement les tests Pyth
cargo test pyth

# Exécuter les tests de types Paradex
cargo test paradex_orderbook
```

### Manual Verification

1. **Compilateur** : `cargo build --release` doit réussir
2. **Logs au démarrage** : Vérifier log `INFO` avec le taux Pyth initial
3. **Comparaison prix** : Lancer le monitor et vérifier que les spreads sont cohérents (pas de ~0.03% de biais systématique)

## Additional Context

### API Reference

**Pyth Hermes - USDC/USD:**
```
GET https://hermes.pyth.network/v2/updates/price/latest?ids[]=eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a
```

**Réponse:**
```json
{
  "parsed": [{
    "price": { "price": "99970000", "expo": -8 }
  }]
}
```

### Dependencies

- `reqwest` (déjà présent)
- Aucune nouvelle dépendance

### Notes

- Les prix WS Paradex sont en USD → convertir
- Les `average_entry_price` positions sont déjà en USDC → ne pas convertir
- Init rate à 1.0 si Pyth indisponible au premier appel

## Red Team Hardening (Applied)

| Risque | Contre-mesure |
|--------|---------------|
| Rate stale pendant trop longtemps | Refresh 15min au lieu de 1h |
| Valeur aberrante de Pyth | Validation bounds `[0.90, 1.10]` |
| Pyth down au démarrage | Retry 3x avec backoff |
| Depeg non détecté | Log WARN si rate change >0.5% |
