# ADR-0001: Record Architecture Decisions

## Status

Accepted

## Context

The Crucible project needs a lightweight, version-controlled method for capturing important architectural decisions. Without a written record, the rationale behind design choices is lost over time, making onboarding harder and increasing the risk of repeating past mistakes.

## Decision

We will use Architecture Decision Records (ADRs), as described by Michael Nygard (http://thinkrelevance.com/blog/2011/11/15/documenting-architecture-decisions).

Each ADR is a short Markdown file stored in `docs/adr/` with a filename format of `NNNN-title-with-dashes.md`. Every ADR contains:

- **Title** — A short description of the decision.
- **Status** — One of: Proposed, Accepted, Deprecated, Superseded.
- **Context** — The problem or forces that motivated the decision.
- **Decision** — The chosen approach.
- **Consequences** — Trade-offs, benefits, and risks.
- **Alternatives Considered** — Other approaches that were explored.

The index of ADRs is maintained in `docs/adr/README.md`.

## Consequences

- Architectural decisions are permanently recorded and discoverable by all contributors.
- Proposing a new ADR requires a pull request with the same review process as code changes.
- ADRs should be updated (or superseded by a new ADR) when the architecture evolves.

## Alternatives Considered

- **Wiki / Confluence** — Not version-controlled; drifts from the codebase.
- **Code comments** — Too fragmented; no overall picture.
- **No documentation** — Leads to knowledge loss and inconsistent contributions.
