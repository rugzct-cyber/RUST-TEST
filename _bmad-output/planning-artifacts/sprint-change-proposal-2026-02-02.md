# Sprint Change Proposal - Epic 6: Bot Automation & Integration

**Date:** 2026-02-02  
**Auteur:** Antigravity (Correct-Course Workflow)  
**Approuvé par:** rugz  
**Type de changement:** Ajout d'Epic (Direct Adjustment)

---

## 1. Issue Summary

### Problème Identifié

**Story Déclencheuse:** Story 4.3 (Configuration des Credentials via .env) - actuellement en "review"

**Catégorie:** Misunderstanding of original requirements

**Problem Statement:**

> Le PRD exige explicitement "Exécution automatique" comme Success Criteria #1 (ligne 34), mais les Epics 1-5 décomposent le système en **composants isolés** (monitoring, execution, state) SANS les intégrer en un bot automatique.
>
> **Gap Critique:** Aucun epic ne connecte spread detection → auto-trigger execution → state persistence dans un runtime unifié.

### Contexte de Découverte

Lors de l'implémentation de Story 4.3, rugz a identifié que:
- Les credentials se chargent correctement via `.env` ✅
- Les composants fonctionnent séparément (Epic 1: monitor, Epic 2: execution) ✅
- **MAIS** `cargo run` ne lance PAS un bot automatique ❌
- Seulement des binaires de test manuels (`monitor.rs`, `delta_neutral_cycle.rs`) existent

### Evidence

**PRD Success Criteria (ligne 34):**
```
"Exécution automatique : Le bot détecte et exécute des trades delta-neutral 
sans intervention manuelle"
```

**PRD Journey 1 - Happy Path (lignes 97-111):**
```
"Rugz lance le bot avec `cargo run --release`"
"Le bot détecte l'opportunité et exécute simultanément..."
```

**État Actuel:**
- `main.rs`: TODOs seulement, pas d'intégration
- `monitor.rs`: Affiche spreads, n'exécute PAS
- `delta_neutral_cycle.rs`: Exécute, mais **déclenchement manuel** requis

**Conclusion:** Le PRD demande automation, les Epics construisent components, mais il manque l'**integration layer**.

---

## 2. Impact Analysis

### 2.1 Epic Impact

**Epic Actuel (Epic 4 - Configuration & Operations):**
- ✅ Peut se compléter EXACTEMENT comme prévu
- ✅ Stories 4.1-4.6 restent inchangées
- ✅ Story 4.3 infrastructure `.env` prête pour Epic 6

**Epics 0-5 (Existants):**
- ✅ **Aucun changement requis** - tous restent valides
- ✅ Ces epics construisent les building blocks utilisés par Epic 6
- ✅ Séquence 0→1→2→3→4→5 inchangée

**Nouvel Epic Requis:**
```
➕ Epic 6: Bot Automation & Integration
   Outcome: `cargo run` → bot trade automatiquement
   Stories: 6.1, 6.2, 6.3, 6.5 (4 stories)
   FRs couverts: Integration de FR1-FR21
```

**Justification:**
- PRD ligne 34: "Exécution automatique" = SUCCESS CRITERIA #1
- Epics 1-5: Composants séparés ≠ bot intégré
- Epic 6 comble le gap: composants → système unifié

**Séquence Finale:**
```
Epic 0 → Epic 1 → Epic 2 → Epic 3 → Epic 4 → Epic 5 → [NEW] Epic 6
               ↓         ↓         ↓         ↓
               └─────────┴─────────┴─────────┴──> Requis pour Epic 6
```

### 2.2 Story Impact

**Stories Existantes:** Aucun changement

**Nouvelles Stories (Epic 6):**

| Story | Nom | Objectif |
|-------|-----|----------|
| 6.1 | Main Runtime Integration | Wire credentials, créer adapters, setup monitoring loop |
| 6.2 | Automatic Delta-Neutral Execution | Auto-trigger execution quand spread ≥ threshold |
| 6.3 | Automatic Position Monitoring & Exit | Auto-close positions quand spread ≤ exit threshold |
| 6.5 | End-to-End Integration Test | Test automatisé du cycle complet |

**Story 6.4 (Multi-Pair) SKIPPED:** Déféré à Phase 4 PRD (ligne 82)

### 2.3 Artifact Conflicts

**PRD:**
- ✅ Aucun conflit
- ✅ Epic 6 **SATISFAIT** Success Criteria "Exécution automatique"
- ✅ Epic 6 **IMPLÉMENTE** Journey 1 (Happy Path)

**Architecture:**
- ✅ Supporte Epic 6
- ⚠️ Ajustement mineur: Documenter `ExecutionEngine` orchestration layer
- ⚠️ Ajustement mineur: Détailler `main.rs` runtime flow

**Tests:**
- ➕ Story 6.5 ajoute test E2E (intégration complète)
- ✅ Tests existants (Epics 1-5) continuent de fonctionner

### 2.4 Technical Impact

**Code à Créer:**
- `src/core/execution_engine.rs` - Orchestration logic (NEW)
- `src/main.rs` - Production runtime (COMPLETE scaffold)
- `tests/integration/full_cycle.rs` - E2E test (NEW)

**Code Existant Utilisé:**
- ✅ `VestConfig::from_env()` (Story 4.3)
- ✅ `ParadexConfig::from_env()` (Story 4.3)
- ✅ `SpreadCalculator` (Epic 1)
- ✅ `DeltaNeutralExecutor` (Epic 2)
- ✅ `StateManager` (Epic 3)

**Dépendances:**
- Epic 6 **REQUIERT** Epics 1, 2, 3, 4 complétés
- Epic 5 (Logging) peut être parallèle

---

## 3. Recommended Approach

### Option Sélectionnée: **Option 1 - Direct Adjustment**

**Approche:** Ajouter Epic 6 avec 4 nouvelles stories (6.1, 6.2, 6.3, 6.5)

**Rationale:**

✅ **Effort:** Medium - Réutilise tous les composants Epics 1-5  
✅ **Risque:** Low - Architecture supporte déjà, juste orchestration  
✅ **Timeline:** +1 Epic (4 stories) après Epic 5  
✅ **Team Momentum:** Positif - complète le système, satisfaction visible  
✅ **Business Value:** High - Atteint Success Criteria PRD  

**Alternatives Considérées:**

❌ **Option 2: Rollback** - Non viable
- Aucune story existante n'est incorrecte
- Epics 1-5 sont tous corrects et nécessaires
- Rollback ne résoudrait pas le gap d'intégration

❌ **Option 3: MVP Review** - Non nécessaire
- MVP PRD reste achievable
- Aucune réduction de scope requise
- Epic 6 fait PARTIE du MVP (Success Criteria #1)

### Effort Estimate

**Epic 6 Effort:**
- Story 6.1: 3-5 heures (wiring existant)
- Story 6.2: 5-8 heures (orchestration logic)
- Story 6.3: 3-5 heures (monitoring + auto-close)
- Story 6.5: 5-8 heures (E2E test)

**Total Epic 6:** 16-26 heures (~2-3 jours dev)

**Risk Level:** Low (réutilise code testé)

---

## 4. Detailed Change Proposals

### 4.1 Changes to Epics Document

**File:** `_bmad-output/planning-artifacts/epics.md`

**Section:** Epic List (after Epic 5)

**OLD:**
```markdown
## Epic 5: Observability & Logging

[...existing content...]

(END OF FILE)
```

**NEW:**
```markdown
## Epic 5: Observability & Logging

[...existing content...]

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
```

**Rationale:** Ajoute Epic 6 requis pour satisfaire PRD Success Criteria "Exécution automatique"

---

### 4.2 Changes to Sprint Status

**File:** `_bmad-output/implementation-artifacts/sprint-status.yaml`

**Section:** Epics list

**OLD:**
```yaml
epics:
  - id: epic-0
    # ...
  - id: epic-5
    title: "Observability & Logging"
    status: backlog
    # ...
# (no epic-6)
```

**NEW:**
```yaml
epics:
  - id: epic-0
    # ...
  - id: epic-5
    title: "Observability & Logging"
    status: backlog
    # ...
  - id: epic-6
    title: "Bot Automation & Integration"
    status: backlog
    stories:
      - id: "6-1"
        title: "Main Runtime Integration"
        status: backlog
      - id: "6-2"
        title: "Automatic Delta-Neutral Execution"
        status: backlog
      - id: "6-3"
        title: "Automatic Position Monitoring & Exit"
        status: backlog
      - id: "6-5"
        title: "End-to-End Integration Test"
        status: backlog
```

**Rationale:** Ajoute Epic 6 au tracking sprint avec 4 nouvelles stories

---

### 4.3 Changes to Architecture (Optional Enhancement)

**File:** `_bmad-output/planning-artifacts/architecture.md`

**Section:** Add new section "Runtime Orchestration"

**NEW SECTION (optional, pour clarté):**
```markdown
## Runtime Orchestration (Epic 6)

### ExecutionEngine

**Responsabilité:** Orchestrer le cycle automatique spread detection → execution → monitoring

**Pattern:** Event-driven orchestration avec channels Tokio

**Integration:**
- ✅ Consomme spreads depuis `SpreadCalculator` (Epic 1)
- ✅ Déclenche `DeltaNeutralExecutor` (Epic 2)
- ✅ Persiste via `StateManager` (Epic 3)
- ✅ Utilise config YAML/env (Epic 4)
- ✅ Logs via tracing (Epic 5)

**Flow:**
```
┌─────────────────────────────────────────────┐
│              main.rs Runtime                │
└─────────────────┬───────────────────────────┘
                  ↓
       ┌──────────────────────┐
       │  ExecutionEngine     │
       └──────────┬───────────┘
                  ↓
    ┌─────────────┴──────────────┐
    ↓                            ↓
SpreadCalculator          StateManager
    ↓                            ↓
DeltaNeutralExecutor    Supabase Sync
```
```

**Rationale:** Documente l'orchestration layer Epic 6 dans l'architecture

---

## 5. Implementation Handoff

### 5.1 Change Scope Classification

**Scope:** **Minor** - Direct implementation par dev team

**Justification:**
- Ajoute 1 epic avec 4 stories
- Réutilise composants existants (Epics 1-5)
- Aucun changement PRD/Architecture majeur
- Pattern bien défini (orchestration)

### 5.2 Handoff Recipients

**Primary:** Développeur (rugz via workflows BMAD)

**Workflows Recommandés:**
1. Mettre à jour `epics.md` et `sprint-status.yaml` manuellement
2. Lancer `/create-story` pour Story 6.1 (première story Epic 6)
3. Lancer `/dev-story` pour implémenter Story 6.1
4. Répéter pour Stories 6.2, 6.3, 6.5

**Secondary:** Aucun (PO/SM/PM pas nécessaires pour ajout mineur)

### 5.3 Success Criteria

**Epic 6 est considéré complet quand:**
- ✅ `cargo run` lance le bot automatique
- ✅ Bot monitore spreads en continu
- ✅ Bot exécute delta-neutral automatiquement quand threshold dépassé
- ✅ Bot ferme positions automatiquement quand exit threshold atteint
- ✅ Test E2E passe sur CI/CD
- ✅ PRD Success Criteria "Exécution automatique" satisfait

### 5.4 Next Steps

**Immediate (après approbation):**
1. ✅ Mettre à jour `epics.md` avec Epic 6 (ajouter section)
2. ✅ Mettre à jour `sprint-status.yaml` (ajouter epic-6 + 4 stories)
3. ✅ Optionnel: Mettre à jour `architecture.md` (documenter orchestration)

**Séquence d'Implémentation:**
1. Compléter Epic 4 (actuellement in-progress)
2. Implémenter Epic 5 (si pas déjà fait)
3. Implémenter Epic 6 stories:
   - 6.1 (Runtime) → 6.2 (Auto-exec) → 6.3 (Auto-close) → 6.5 (E2E test)

**Timeline Estimée:**
- Update docs: 30 mins
- Epic 6 implémentation: 2-3 jours dev

---

## 6. Approval

**Status:** ✅ **APPROVED** by rugz (2026-02-02)

**Approvals:**
- [x] User (rugz): Confirmé Option 1, stories 6.1-6.3, 6.5
- [x] Technical feasibility: Architecture supporte
- [x] PRD alignment: Satisfait Success Criteria

**Conditions:** Aucune

**Date d'Approbation:** 2026-02-02

---

## Summary

**Change Type:** Ajout d'Epic 6 (Direct Adjustment)

**Impact:**
- ➕ 1 nouveau epic
- ➕ 4 nouvelles stories
- ✅ Epics 0-5 inchangés
- ✅ PRD/Architecture alignés

**Business Value:** Atteint PRD Success Criteria #1 "Exécution automatique"

**Timeline:** +2-3 jours dev après Epic 5

**Risk:** Low - Réutilise composants testés

---

**Document généré par:** Antigravity (Correct-Course Workflow)  
**Date:** 2026-02-02  
**Version:** 1.0
