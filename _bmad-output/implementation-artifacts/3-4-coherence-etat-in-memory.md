# Story 3.4: Cohérence État In-Memory

Status: review

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **opérateur**,
I want que l'état in-memory reste cohérent avec Supabase,
So that les décisions du bot soient basées sur des données fiables.

## Acceptance Criteria

1. **Given** des positions en mémoire et dans Supabase
   **When** une position est mise à jour (close, partial fill)
   **Then** l'état in-memory est mis à jour immédiatement
   **And** Supabase est synchronisé de manière asynchrone
   
2. **Given** une mise à jour de position
   **When** la synchronisation Supabase échoue
   **Then** un retry est effectué (géré par le caller)
   **And** l'erreur est propagée pour permettre au caller de décider (retry/abort)

3. **Given** une position supprimée in-memory
   **When** `remove_position()` est appelé
   **Then** la position est supprimée de Supabase via DELETE
   **And** un log `[INFO] Position removed from Supabase` est émis

4. **Given** une position mise à jour in-memory
   **When** `update_position()` est appelé
   **Then** la position est mise à jour dans Supabase via PATCH
   **And** un log `[INFO] Position updated in Supabase` est émis

## Tasks / Subtasks

- [x] **Task 1**: Analyser le code existant et patterns (AC: all)
  - [x] Subtask 1.1: Lire `src/core/state.rs` lignes 435-442 — `update_position()` stub actuel
  - [x] Subtask 1.2: Lire `src/core/state.rs` lignes 451-454 — `remove_position()` stub actuel
  - [x] Subtask 1.3: Lire Stories 3.2 et 3.3 — pattern HTTP POST/GET Supabase
  - [x] Subtask 1.4: Identifier pattern de filtering Supabase: `?id=eq.{uuid}`
  - [x] Subtask 1.5: Vérifier `PositionUpdate` struct (lignes 227-234) — déjà défini

- [x] **Task 2**: Implémenter `update_position()` avec PATCH Supabase (AC: #1, #2, #4)
  - [x] Subtask 2.1: Construire PATCH URL: `{supabase_url}/rest/v1/positions?id=eq.{uuid}`
  - [x] Subtask 2.2: Ajouter headers (déjà configurés dans client): `apikey`, `Authorization`, `Content-Type`
  - [x] Subtask 2.3: Ajouter header `Prefer: return=minimal` pour optimiser bandwidth
  - [x] Subtask 2.4: Envoyer PATCH via `reqwest::Client::patch().json(&updates).send().await`
  - [x] Subtask 2.5: Vérifier statut 204 NO_CONTENT ou 200 OK → succès
  - [x] Subtask 2.6: Logger `info!(position_id, remaining_size, status, "Position updated in Supabase")`

- [x] **Task 3**: Gérer les erreurs HTTP pour `update_position()` (AC: #2)
  - [x] Subtask 3.1: Status 204/200 → succès, retourne `Ok(())`
  - [x] Subtask 3.2: Status 401 Unauthorized → `StateError::DatabaseError` avec message credentials
  - [x] Subtask 3.3: Status 404 Not Found → `StateError::NotFound` (position déjà supprimée)
  - [x] Subtask 3.4: Status 4xx/5xx → `StateError::DatabaseError` avec status + body
  - [x] Subtask 3.5: Timeout/Network → `StateError::NetworkError` (auto-convert via `#[from]`)
  - [x] Subtask 3.6: Logger `error!` pour toutes les erreurs avec contexte complet

- [x] **Task 4**: Implémenter `remove_position()` avec DELETE Supabase (AC: #3)
  - [x] Subtask 4.1: Construire DELETE URL: `{supabase_url}/rest/v1/positions?id=eq.{uuid}`
  - [x] Subtask 4.2: Envoyer DELETE via `reqwest::Client::delete().send().await`
  - [x] Subtask 4.3: Vérifier statut 204 NO_CONTENT ou 200 OK → succès
  - [x] Subtask 4.4: Logger `info!(position_id, "Position removed from Supabase")`

- [x] **Task 5**: Gérer les erreurs HTTP pour `remove_position()` (AC: #3)
  - [x] Subtask 5.1: Status 204/200 → succès, retourne `Ok(())`
  - [x] Subtask 5.2: Status 401 Unauthorized → `StateError::DatabaseError`
  - [x] Subtask 5.3: Status 404 Not Found → `Ok()` (idempotent - déjà supprimé)
  - [x] Subtask 5.4: Status 4xx/5xx → `StateError::DatabaseError` avec status + body
  - [x] Subtask 5.5: Timeout/Network → `StateError::NetworkError`
  - [x] Subtask 5.6: Logger `error!` pour toutes les erreurs avec contexte

- [x] **Task 6**: Cas désactivé pour les deux méthodes (AC: all)
  - [x] Subtask 6.1: Si `supabase_client.is_none()` → retourner immédiatement `Ok(())`
  - [x] Subtask 6.2: Logger `debug!("Supabase disabled, position update/removal not synced")`

- [x] **Task 7**: Tests unitaires (AC: all)
  - [x] Subtask 7.1: `test_update_position_disabled()` — config disabled → Ok() sans HTTP
  - [x] Subtask 7.2: `test_update_position_success()` — mock PATCH 204 NO_CONTENT
  - [x] Subtask 7.3: `test_update_position_unauthorized()` — mock 401 Unauthorized
  - [x] Subtask 7.4: `test_update_position_not_found()` — mock 404 Not Found
  - [x] Subtask 7.5: `test_remove_position_disabled()` — config disabled → OK() sans HTTP
  - [x] Subtask 7.6: `test_remove_position_success()` — mock DELETE 204 NO_CONTENT
  - [x] Subtask 7.7: `test_remove_position_unauthorized()` — mock 401 Unauthorized
  - [x] Subtask 7.8: `test_remove_position_idempotent()` — mock 404 → Ok() (idempotent)

- [x] **Task 8**: Validation finale (AC: all)
  - [x] Subtask 8.1: `cargo build` compile sans warnings
  - [x] Subtask 8.2: `cargo clippy --all-targets -- -D warnings` propre
  - [x] Subtask 8.3: `cargo test` tous les tests passent (226 au total: 218 + 8 nouveaux)
  - [x] Subtask 8.4: Vérifier logs avec `RUST_LOG=debug` — logs structurés présents

## Definition of Done Checklist

- [x] Code compiles sans warnings (`cargo build`)
- [x] Clippy propre (`cargo clippy --all-targets -- -D warnings`)
- [x] Tests passent (`cargo test`)
- [x] `update_position()` PATCH vers `/rest/v1/positions?id=eq.{uuid}` fonctionne
- [x] `remove_position()` DELETE vers `/rest/v1/positions?id=eq.{uuid}` fonctionne
- [x] Gestion erreurs 401 Unauthorized (invalid credentials)
- [x] Gestion erreurs 404 Not Found (idempotent pour DELETE, NotFound pour UPDATE)
- [x] Gestion erreurs réseau et timeouts
- [x] Cas Supabase désactivé géré correctement (retourne Ok sans HTTP call)
- [x] 8 tests unitaires ajoutés (total: 226 tests)
- [x] Logs structurés `info!`, `warn!`, `error!` présents
- [x] Documentation inline mise à jour

## Dev Notes

### Architecture Pattern — In-Memory + Supabase Sync

**CRITICAL CONTEXT from Epic 3:**

Story 3.4 implémente la **synchronisation Supabase** pour les opérations UPDATE et DELETE. La cohérence in-memory est gérée par le **caller** (runtime.rs ou code métier), pas par StateManager.

**Séparation des responsabilités:**

```
┌─────────────────────────────────────┐
│   Caller (runtime.rs, etc.)         │
│   - Gère HashMap<Uuid, Position>    │
│   - Met à jour in-memory d'abord    │
│   - Appelle StateManager après      │
└─────────────────────────────────────┘
              ↓
┌─────────────────────────────────────┐
│   StateManager (state.rs)           │
│   - Sync Supabase uniquement        │
│   - PATCH/DELETE HTTP operations    │
│   - Error handling et logging       │
└─────────────────────────────────────┘
```

**Pourquoi ce pattern ?**

- ✅ Cohérent avec Stories 3.2/3.3 (StateManager = pure Supabase operations)
- ✅ Évite duplication état (AppState existe déjà avec HashMap)
- ✅ Caller contrôle retry logic et error recovery
- ✅ Simple et testable

**Note importante:**
- Story 3.4 ne gère PAS le retry automatique (AC#2 interprété comme: caller gère retry)
- Epic 4 (FR16-18) ajoutera resilience globale si nécessaire

### Code Location & Existing Structure

**Fichier à modifier:** `src/core/state.rs`

**Lignes cibles:**
- `update_position()`: lignes 435-442 (stub actuel)
- `remove_position()`: lignes 451-454 (stub actuel)

**Structure cible:** `StateManager` (lignes 236-455)
- Champ: `supabase_url: String` (ajouté Story 3.2)
- Champ: `supabase_client: Option<reqwest::Client>` (ajouté Story 3.2)
- Méthodes implémentées: `save_position()` (Story 3.2), `load_positions()` (Story 3.3)
- Méthodes stub: `update_position()`, `remove_position()` **(Story 3.4 - à implémenter)**

### Supabase REST API Integration

#### UPDATE Operation — PATCH Endpoint

**Endpoint URL Pattern:**
```
PATCH https://{project}.supabase.co/rest/v1/positions?id=eq.{uuid}
```

**Required Headers (déjà configurés dans client):**
```rust
headers: {
  "apikey": "{SUPABASE_ANON_KEY}",
  "Authorization": "Bearer {SUPABASE_ANON_KEY}",
  "Content-Type": "application/json",
  "Prefer": "return=minimal",  // Optimisation: pas de body response
}
```

**Request Body:**
```json
{
  "remaining_size": 0.25,
  "status": "PartialClose"
}
```

**Expected Response:**
- **204 NO_CONTENT** → Succès (pas de body, juste confirmation)
- **200 OK** → Succès (avec body, mais on l'ignore avec `Prefer: return=minimal`)
- **401 Unauthorized** → Invalid API key
- **404 Not Found** → Position n'existe pas (déjà supprimée ?)
- **500 Server Error** → Supabase internal error

#### DELETE Operation — DELETE Endpoint

**Endpoint URL Pattern:**
```
DELETE https://{project}.supabase.co/rest/v1/positions?id=eq.{uuid}
```

**Required Headers (déjà configurés):**
```rust
headers: {
  "apikey": "{SUPABASE_ANON_KEY}",
  "Authorization": "Bearer {SUPABASE_ANON_KEY}",
}
```

**Expected Response:**
- **204 NO_CONTENT** → Succès (suppression confirmée)
- **200 OK** → Succès (alternative response)
- **401 Unauthorized** → Invalid API key
- **404 Not Found** → Position déjà supprimée (traiter comme succès - idempotent)
- **500 Server Error** → Supabase internal error

### Implementation Pattern (Based on Stories 3.2/3.3)

#### Pattern `update_position()`

**Template (suivre exactement Stories 3.2/3.3):**

```rust
pub async fn update_position(
    &self,
    id: Uuid,
    updates: PositionUpdate,
) -> Result<(), StateError> {
    // Handle disabled case
    if self.supabase_client.is_none() {
        tracing::debug!("Supabase disabled, position update not synced");
        return Ok(());
    }
    
    let client = self.supabase_client.as_ref().unwrap();
    let url = format!("{}/rest/v1/positions?id=eq.{}", self.supabase_url, id);
    
    // Send PATCH request
    let response = client
        .patch(&url)
        .header("Prefer", "return=minimal")
        .json(&updates)
        .send()
        .await?;  // NetworkError auto-converted via #[from]
    
    // Handle HTTP status codes
    match response.status() {
        reqwest::StatusCode::NO_CONTENT | reqwest::StatusCode::OK => {
            tracing::info!(
                position_id = %id,
                remaining_size = ?updates.remaining_size,
                status = ?updates.status,
                "Position updated in Supabase"
            );
            Ok(())
        }
        reqwest::StatusCode::UNAUTHORIZED => {
            let err_msg = "Invalid Supabase credentials".to_string();
            tracing::error!(
                position_id = %id,
                "Failed to update position in Supabase: {}", err_msg
            );
            Err(StateError::DatabaseError(err_msg))
        }
        reqwest::StatusCode::NOT_FOUND => {
            let err_msg = format!("Position not found: {}", id);
            tracing::warn!(
                position_id = %id,
                "Position not found in Supabase (may have been deleted)"
            );
            Err(StateError::NotFound)
        }
        status => {
            let body = response.text().await.unwrap_or_else(|_| "<no body>".to_string());
            let err_msg = format!("Supabase error {}: {}", status, body);
            tracing::error!(
                position_id = %id,
                status = %status,
                response_body = %body,
                "Failed to update position in Supabase"
            );
            Err(StateError::DatabaseError(err_msg))
        }
    }
}
```

#### Pattern `remove_position()`

**Template:**

```rust
pub async fn remove_position(&self, id: Uuid) -> Result<(), StateError> {
    // Handle disabled case
    if self.supabase_client.is_none() {
        tracing::debug!("Supabase disabled, position removal not synced");
        return Ok(());
    }
    
    let client = self.supabase_client.as_ref().unwrap();
    let url = format!("{}/rest/v1/positions?id=eq.{}", self.supabase_url, id);
    
    // Send DELETE request
    let response = client
        .delete(&url)
        .send()
        .await?;
    
    // Handle HTTP status codes
    match response.status() {
        reqwest::StatusCode::NO_CONTENT | reqwest::StatusCode::OK => {
            tracing::info!(
                position_id = %id,
                "Position removed from Supabase"
            );
            Ok(())
        }
        reqwest::StatusCode::UNAUTHORIZED => {
            let err_msg = "Invalid Supabase credentials".to_string();
            tracing::error!(
                position_id = %id,
                "Failed to remove position from Supabase: {}", err_msg
            );
            Err(StateError::DatabaseError(err_msg))
        }
        reqwest::StatusCode::NOT_FOUND => {
            // NOT_FOUND is acceptable for DELETE (idempotent operation)
            tracing::info!(
                position_id = %id,
                "Position already removed from Supabase (idempotent)"
            );
            Ok(())
        }
        status => {
            let body = response.text().await.unwrap_or_else(|_| "<no body>".to_string());
            let err_msg = format!("Supabase error {}: {}", status, body);
            tracing::error!(
                position_id = %id,
                status = %status,
                response_body = %body,
                "Failed to remove position from Supabase"
            );
            Err(StateError::DatabaseError(err_msg))
        }
    }
}
```

### Error Handling Patterns

**Pattern existant (StateError enum)** — déjà défini, pas de changement:

```rust
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("Database error: {0}")]
    DatabaseError(String),
    
    #[error("Position not found")]
    NotFound,  // Utilisé pour UPDATE si 404
    
    #[error("Invalid data: {0}")]
    InvalidData(String),
    
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),  // Auto-conversion
}
```

**Mapping HTTP Status → StateError:**

| HTTP Status | update_position() | remove_position() |
|-------------|-------------------|-------------------|
| 204/200 | Ok(()) | Ok(()) |
| 401 | DatabaseError | DatabaseError |
| 404 | StateError::NotFound | Ok() (idempotent) |
| 4xx/5xx | DatabaseError | DatabaseError |
| Network | NetworkError | NetworkError |

**Rationale 404 différent:**
- **UPDATE**: 404 = erreur (position devrait exister) → `StateError::NotFound`
- **DELETE**: 404 = succès (déjà supprimé, opération idempotente) → `Ok(())`

### Logging Patterns

**Success logs:**

```rust
// UPDATE success
tracing::info!(
    position_id = %id,
    remaining_size = ?updates.remaining_size,
    status = ?updates.status,
    "Position updated in Supabase"
);

// DELETE success
tracing::info!(
    position_id = %id,
    "Position removed from Supabase"
);

// DELETE idempotent (404)
tracing::info!(
    position_id = %id,
    "Position already removed from Supabase (idempotent)"
);
```

**Error logs:**

```rust
tracing::error!(
    position_id = %id,
    status = %status,
    response_body = %body,
    "Failed to update/remove position from Supabase"
);
```

**Warning logs:**

```rust
// UPDATE 404 (position disparue)
tracing::warn!(
    position_id = %id,
    "Position not found in Supabase (may have been deleted)"
);
```

### Testing Strategy

**Baseline:** 218 tests actuellement (Stories 3.1-3.3 + Code Reviews)

**Nouveaux tests attendus (+8, total: 226):**

#### Tests `update_position()` (4 tests):

1. **`test_update_position_disabled()`** — Config disabled → Ok() sans HTTP call

```rust
#[tokio::test]
async fn test_update_position_disabled() {
    let config = crate::config::SupabaseConfig {
        url: "https://test.supabase.co".to_string(),
        anon_key: "test-key".to_string(),
        enabled: false,
    };
    
    let manager = StateManager::new(config);
    let id = Uuid::new_v4();
    let updates = PositionUpdate {
        remaining_size: Some(0.25),
        status: Some(PositionStatus::PartialClose),
    };
    
    let result = manager.update_position(id, updates).await;
    assert!(result.is_ok());
    // Should NOT make HTTP request
}
```

2. **`test_update_position_success()`** — Mock PATCH 204 NO_CONTENT → Ok()

```rust
#[tokio::test]
async fn test_update_position_success() {
    let mut server = mockito::Server::new_async().await;
    let test_id = Uuid::new_v4();
    
    let mock = server.mock("PATCH", format!("/rest/v1/positions?id=eq.{}", test_id).as_str())
        .with_status(204)
        .create_async()
        .await;

    let config = crate::config::SupabaseConfig {
        url: server.url(),
        anon_key: "test-key".to_string(),
        enabled: true,
    };

    let manager = StateManager::new(config);
    let updates = PositionUpdate {
        remaining_size: Some(0.25),
        status: Some(PositionStatus::PartialClose),
    };

    let result = manager.update_position(test_id, updates).await;
    assert!(result.is_ok());
    mock.assert_async().await;
}
```

3. **`test_update_position_unauthorized()`** — Mock 401 → DatabaseError

4. **`test_update_position_not_found()`** — Mock 404 → StateError::NotFound

#### Tests `remove_position()` (4 tests):

5. **`test_remove_position_disabled()`** — Config disabled → Ok() sans HTTP

6. **`test_remove_position_success()`** — Mock DELETE 204 NO_CONTENT → Ok()

7. **`test_remove_position_unauthorized()`** — Mock 401 → DatabaseError

8. **`test_remove_position_idempotent()`** — Mock 404 → Ok() (idempotent)

```rust
#[tokio::test]
async fn test_remove_position_idempotent() {
    let mut server = mockito::Server::new_async().await;
    let test_id = Uuid::new_v4();
    
    let mock = server.mock("DELETE", format!("/rest/v1/positions?id=eq.{}", test_id).as_str())
        .with_status(404)
        .with_body("Not Found")
        .create_async()
        .await;

    let config = crate::config::SupabaseConfig {
        url: server.url(),
        anon_key: "test-key".to_string(),
        enabled: true,
    };

    let manager = StateManager::new(config);
    let result = manager.remove_position(test_id).await;
    
    // Should return Ok (idempotent - already deleted)
    assert!(result.is_ok());
    mock.assert_async().await;
}
```

### Dependencies

**Vérifier dans Cargo.toml (déjà présents via Stories 3.1-3.3):**

```toml
[dependencies]
reqwest = { version = "0.11", features = ["json"] }  # Déjà présent
tokio = { version = "1", features = ["full"] }        # Déjà présent
serde = { version = "1.0", features = ["derive"] }    # Déjà présent
serde_json = "1.0"                                    # Déjà présent
tracing = "0.1"                                       # Déjà présent
uuid = { version = "1.0", features = ["v4", "serde"] } # Déjà présent

[dev-dependencies]
mockito = "1.7"  # Déjà ajouté Story 3.2
```

**Aucune nouvelle dépendance requise.**

### Previous Story Intelligence (Stories 3.1-3.3)

**Story 3.1 — Foundation:**
- `PositionState` struct créé avec Natural Key fields
- `StateManager` struct créé avec `supabase_url` et `supabase_client`
- `StateError` enum pour error handling (includes `NotFound` variant)
- Tests baseline: 206 tests

**Story 3.2 — Save Logic:**
- `save_position()` implémenté avec POST Supabase
- Pattern HTTP status code handling établi
- Pattern logging structuré établi
- Pattern mockito pour tests HTTP
- Tests: 210 total (206 + 4 nouveaux)

**Story 3.3 — Load Logic:**
- `load_position()` implémenté avec GET filtering `?status=eq.Open`
- Gestion erreurs 401, 404, network
- Tests: 218 total (210 + 8 nouveaux)
- Code Review fixes: timeout 10s, English messages, JSON parsing errors

**Learnings à appliquer:**

1. **HTTP Pattern:** Story 3.4 suit le MÊME pattern que Stories 3.2/3.3
   - Vérifier `supabase_client.is_none()` en premier
   - Construire URL avec filtering Supabase `?id=eq.{uuid}`
   - Match sur `response.status()` pour error handling
   - Auto-convert `reqwest::Error` → `StateError::NetworkError`

2. **Logging Pattern:** Utiliser `tracing` macros avec structured fields
   - `info!` pour succès avec position_id
   - `debug!` pour cas désactivé
   - `warn!` pour situations anormales mais récupérables (404 UPDATE)
   - `error!` pour échecs avec contexte complet

3. **Testing Pattern:** Mockito pour HTTP mocking
   - `Server::new_async().await` pour mock server
   - `.mock("PATCH"|"DELETE", "/rest/v1/positions?id=eq.{uuid}")`
   - `.with_status(204|401|404)` + `.with_body(json_string)` si besoin
   - `.create_async().await`

4. **Idempotence Pattern:**
   - DELETE 404 → Ok() (position déjà supprimée, pas une erreur)
   - UPDATE 404 → StateError::NotFound (position devrait exister)

5. **Timeout Configuration:**
   - Déjà configuré dans `StateManager::new()` (10s timeout, Story 3.3 Code Review)
   - Pas besoin de le reconfigurer

### FR Coverage

Story 3.4 couvre **FR12: Maintien état in-memory cohérent**

Alignement avec NFR10: "State recovery — Positions restaurées après restart" (complète Epic 3)

### Integration avec le reste du système

**Caller attendu:** Code métier dans `runtime.rs` ou modules d'exécution

**Pattern d'utilisation prévu:**

```rust
// Exemple: Partial close d'une position
let position_id = Uuid::parse_str("...").unwrap();

// 1. Update in-memory state FIRST (caller responsibility)
if let Some(pos) = in_memory_positions.get_mut(&position_id) {
    pos.remaining_size = 0.25;
    pos.status = PositionStatus::PartialClose;
    
    // 2. Sync to Supabase (async)
    let updates = PositionUpdate {
        remaining_size: Some(0.25),
        status: Some(PositionStatus::PartialClose),
    };
    
    match state_manager.update_position(position_id, updates).await {
        Ok(()) => tracing::info!("Position synced to Supabase"),
        Err(e) => {
            tracing::error!("Failed to sync position: {}", e);
            // Caller decides: retry, abort, or continue
        }
    }
}

// Exemple: Full close (suppression)
if let Some(_) = in_memory_positions.remove(&position_id) {
    match state_manager.remove_position(position_id).await {
        Ok(()) => tracing::info!("Position removed from Supabase"),
        Err(e) => tracing::error!("Failed to remove position: {}", e),
    }
}
```

**Note:** Story 3.4 ne gère PAS la HashMap in-memory. Le caller gère cela.

### References

- [Source: epics.md#Epic-3 Story 3.4] Requirements de base (AC1-AC3)
- [Source: architecture.md#Data-Architecture] Schema Supabase
- [Source: architecture.md#Error-Handling-Patterns] thiserror patterns
- [Source: architecture.md#Logging-Patterns] tracing macros format
- [Source: 3-1-creation-module-state-persistence.md] StateManager foundation
- [Source: 3-2-sauvegarde-positions-supabase.md] HTTP POST pattern + error handling
- [Source: 3-3-restauration-etat-apres-redemarrage.md] HTTP GET pattern + filtering
- [Source: sprint-status.yaml#L109-111] Natural Key design decision
- [Source: src/core/state.rs#L236-455] StateManager actuel
- [Source: src/core/state.rs#L435-442] update_position() stub
- [Source: src/core/state.rs#L451-454] remove_position() stub
- [Source: src/core/state.rs#L227-234] PositionUpdate struct (déjà défini)

## Dev Agent Record

### Agent Model Used

google/gemini-exp-1206

### Debug Log References

N/A - No errors encountered during implementation

### Completion Notes List

- ✅ Implemented `update_position()` with PATCH HTTP operation to `/rest/v1/positions?id=eq.{uuid}`
- ✅ Implemented `remove_position()` with DELETE HTTP operation to `/rest/v1/positions?id=eq.{uuid}`
- ✅ Followed established patterns from Stories 3.2/3.3: disabled handling, status code matching, logging
- ✅ Idempotent DELETE behavior: 404 NOT_FOUND treated as success (already deleted)
- ✅ UPDATE 404 treated as error (StateError::NotFound) - position should exist
- ✅ Added 8 comprehensive unit tests using mockito for HTTP mocking
- ✅ All validations passed: Build (47s), Clippy (0 warnings), Tests (226/226 passing)
- ✅ Fixed unused variable warning during development

### File List

- Modified: `src/core/state.rs` (implemented `update_position()` and `remove_position()`, added 8 unit tests)
