## Summary

Add a complete issue taxonomy, milestone structure, and backlog management framework for the crucible repository.

## Changes Made

### Labels (27 created)
- **Area** (11): `area:core-api`, `area:backend`, `area:build`, `area:examples`, `area:docs`, `area:testing`, `area:security`, `area:ci`, `area:fixtures`, `area:tokens`, `area:performance`
- **Priority** (4): `priority:P0` through `priority:P3`
- **Type** (6): `type:bug`, `type:feature`, `type:enhancement`, `type:refactor`, `type:chore`, `type:docs`
- **Status** (4): `blocked`, `needs-discussion`, `ready`, `in-progress`
- **Contributor** (2): `good-first-issue`, `help-wanted`

### Milestones (7 created)
| Milestone | Due | Description |
|-----------|-----|-------------|
| `build-green` | 2026-07-09 | Workspace compiles, CI passes, SDK versions unified |
| `core-api` | 2026-07-23 | MockEnv, accounts, tokens, assertion macros |
| `backend-stabilization` | 2026-07-23 | Backend builds, tests pass, core workflows functional |
| `v0.1` | 2026-08-06 | First crates.io release |
| `v0.2` | 2026-08-20 | Cost tracking, snapshots, SimulatedTx |
| `v0.3` | 2026-09-03 | Fixtures DX, time controls, CLI report |
| `v0.4` | 2026-09-17 | Pre-built mocks, soroban-cli, VSCode extension |

### Issues (13 created and labeled)
- 7 tracking issues covering blocker, core API, tokens, docs, backend, security, fixtures
- 6 individual good-first-issue items with detailed descriptions and file references
- Every issue has exactly one area, one priority, one type, and one status

### Repository Files
- `.github/labels.yml` — label definitions with colors and descriptions
- `.github/project-governance.md` — full governance documentation
- `.github/ISSUE_TEMPLATE/config.yml` — blank issue policy
- `.github/ISSUE_TEMPLATE/bug_report.yml` — structured bug report form
- `.github/ISSUE_TEMPLATE/feature_request.yml` — structured feature request form

## Rationale

The repository had a single `Stellar Wave` label for 54 open issues with no milestones, no area/priority/type taxonomy, and no contributor onboarding structure. This made triage, filtering, and release planning impractical. The new taxonomy is based on a thorough analysis of the codebase structure and upstream issue backlog.

## Acceptance Checklist

- [x] Labels created with correct colors and descriptions
- [x] Default GitHub labels removed to avoid confusion
- [x] Milestones created with correct dependency ordering
- [x] Issues classified with exactly one area, priority, type, and status
- [x] Good-first-issue items created with detailed instructions
- [x] Governance document written
- [x] Issue templates guide contributors to proper categorization
- [x] All issues filterable by area
- [x] All issues filterable by priority
- [x] No undefined labels used
- [x] Milestone order reflects practical delivery sequence
