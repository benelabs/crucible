# Project Governance

## Label Definitions

### Area (exactly one per issue)

| Label | Scope | Example files |
|-------|-------|---------------|
| `area:core-api` | Crucible testing library — MockEnv, macros, env, sim, account, cost, prelude | `contracts/crucible/src/{env,macros,sim,account,cost,prelude,lib}.rs` |
| `area:backend` | Axum backend API server — handlers, services, DB, config, workers, telemetry | `backend/src/**` |
| `area:build` | Cargo workspace, Makefile, compilation toolchain, MSRV, feature flags | `Cargo.toml`, `Makefile`, `.cargo/config.toml` |
| `area:examples` | Example Soroban contracts and their test suites | `examples/*/src/**` |
| `area:docs` | READMEs, doc comments, architecture docs, tutorials, API reference | `*.md`, doc comments, `backend/*IMPLEMENTATION*.md` |
| `area:testing` | Test infrastructure and utilities — not the library API itself | `backend/test_utils/`, `backend/tests/`, test patterns |
| `area:security` | Vulnerability scanning, RBAC, audit logging, input validation, rate limiting | `backend/src/services/{security,vulnerability}_scanner.rs`, permissions middleware |
| `area:ci` | GitHub Actions, Dependabot, pipeline scripts, CI config | `.github/workflows/`, `.github/dependabot.yml` |
| `area:fixtures` | `#[fixture]` derive macro, crucible-macros crate, fixture patterns | `crucible-macros/**`, `contracts/crucible/src/fixture.rs` |
| `area:tokens` | `MockToken`, SAC/SEP-41 interface, token helpers | `contracts/crucible/src/token.rs` |
| `area:performance` | Cost tracking, gas estimation, instruction counting, benchmarks, `CostReport` | `contracts/crucible/src/cost.rs`, `backend/benches/**` |

### Priority (exactly one per issue)

| Label | Meaning | Rules |
|-------|---------|-------|
| `priority:P0` | Blocker — blocks release or makes project unusable | One per milestone max. Requires immediate triage. Must have owner. |
| `priority:P1` | High — important for current milestone | Should be actively worked this cycle. Assigned to a milestone. |
| `priority:P2` | Medium — nice-to-have, can slip | Unblocks other work but not urgent. Default for new issues. |
| `priority:P3` | Low — icebox, no immediate timeline | No milestone required. Revisit during backlog grooming. |

### Type (exactly one per issue)

| Label | When to use | Must not co-occur with |
|-------|-------------|------------------------|
| `type:bug` | Something is broken | `type:feature`, `type:enhancement` |
| `type:feature` | Net-new capability or public API | `type:bug`, `type:enhancement`, `type:refactor` |
| `type:enhancement` | Improvement to existing functionality | `type:bug`, `type:feature` |
| `type:refactor` | Internal restructuring, zero behavior change | `type:feature`, `type:enhancement` |
| `type:chore` | Maintenance, deps, version bumps, CI config | All others (chore is exclusive) |
| `type:docs` | Documentation additions or improvements | None (can co-occur) |

### Status (exactly one per issue)

| Label | Meaning | Entry criteria | Exit criteria |
|-------|---------|----------------|---------------|
| `blocked` | Cannot proceed — depends on something else | Must link to blocking item in comment | Close as completed when unblocked |
| `needs-discussion` | Requires design input before implementation | Must have proposed approach comment | Convert to `ready` on consensus |
| `ready` | Triaged, accepted, actionable | Must have area + priority + type set | Move to `in-progress` when work starts |
| `in-progress` | Actively being worked on | Assignee must be set, PR should exist | Move to `ready` if abandoned |

### Contributor

| Label | Rules |
|-------|-------|
| `good-first-issue` | Must co-occur with `help-wanted`. Must have: 3-step description, file links, mentor. Max 5 open. |
| `help-wanted` | May appear without `good-first-issue`. Must have approach comment. |

---

## Milestone Definitions

| Milestone | Depends on | Delivery | Contents |
|-----------|-----------|----------|----------|
| `build-green` | — | 2 weeks | Workspace compiles, CI runs `cargo test`, SDK versions unified, lint passes |
| `core-api` | `build-green` | 2 weeks | MockEnv, pre-funded accounts, MockToken, assertion macros, `assert_reverts!` |
| `backend-stabilization` | `build-green` | 2 weeks (parallel) | Backend compiles, tests pass, DB migrations apply, Docker compose works |
| `v0.1` | `core-api` | 2 weeks | First crates.io release + metadata + changelog |
| `v0.2` | `v0.1` | 2 weeks | Cost tracking, `env.measure()`, snapshots, `SimulatedTx` |
| `v0.3` | `v0.2` | 2 weeks | Fixtures DX, time controls, event captures, CLI report |
| `v0.4` | `v0.3` | 2 weeks | Pre-built mocks, soroban-cli integration, VSCode extension |

---

## Issue Classification Rules

1. **Every issue** must have exactly one `area:*`, one `priority:*`, one `type:*`, and one status label.
2. **`good-first-issue`** must always co-occur with **`help-wanted`**.
3. **`type:feature` and `type:enhancement` are mutually exclusive.** If unsure, default to `enhancement`.
4. **`status:ready`** requires area + priority + type to be set. Automated triage should verify this.
5. **`priority:P0`** automatically implies `status:blocked` until resolved.
6. **No ad-hoc labels.** All labels must be defined in `labels.yml`. Propose new labels via issue discussion.

---

## Contributor Workflow

### First-time contributor

1. Find a `good-first-issue` (all have `help-wanted` too).
2. Read the issue description — it includes file links and a 3-step plan.
3. Ask questions in the issue comments if anything is unclear.
4. Submit a PR with `Closes #N` in the description.
5. Run `cargo fmt` and `cargo clippy -- -D warnings` before requesting review.

### Regular contributor

1. Pick an issue with `status:ready` and assign yourself.
2. Move the issue to `status:in-progress`.
3. Open a draft PR early for visibility.
4. On PR submission, the issue moves to `status:ready` if closed, or stays `in-progress` if follow-up is needed.

### Maintainer

1. Triage new issues weekly: apply area + priority + type + status.
2. Move `needs-discussion` to `ready` when consensus is reached.
3. Close milestones when all issues are closed and CI is green.
4. Cut releases per the checklist in `RELEASE_CHECKLIST.md`.

---

## Examples

### Filing a new bug

```
Title: MockEnv::simulate panics on empty contract calls

Labels: area:core-api, priority:P1, type:bug, status:needs-discussion
Body: Steps to reproduce, expected behavior, actual behavior, environment
```

### Filing a feature request

```
Title: Add MockToken::burn helper

Labels: area:tokens, priority:P2, type:feature, status:needs-discussion
Body: Proposed API, use case, alternatives considered
```

### Good first issue

```
Title: GFI: Derive Clone on AccountHandle

Labels: area:core-api, priority:P3, type:enhancement,
       good-first-issue, help-wanted, status:ready
Body: File links, 3-step plan, verification instructions
```
