# Security Policy

## Supply-chain security

Crucible's Rust workspace pulls in a large dependency graph (the backend
alone has many transitive crates, plus proc-macro build dependencies). To keep
that surface trustworthy we enforce an automated supply-chain policy in CI.

### What is checked

The **Supply Chain Security** GitHub Actions workflow
([.github/workflows/supply-chain.yml](.github/workflows/supply-chain.yml))
runs two complementary tools on every dependency change and weekly:

| Tool | Purpose |
| --- | --- |
| [`cargo-deny`](https://embarkstudios.github.io/cargo-deny/) | Advisories, license policy, banned/duplicate crates, and source allow-listing — configured in [`deny.toml`](deny.toml). |
| [`cargo-audit`](https://crates.io/crates/cargo-audit) | Cross-checks `Cargo.lock` against the [RUSTSEC advisory database](https://rustsec.org/) for known-vulnerable versions. |

A failure in either job blocks the merge.

### Policy summary

The policy lives in [`deny.toml`](deny.toml) and is enforced, not just
documented. In short:

- **Vulnerabilities** — any crate version with an open RUSTSEC advisory fails
  the build. Yanked crates are rejected.
- **Licenses** — only permissive, OSI-approved licenses are allowed
  (MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, MPL-2.0, Unicode, CC0). A
  dependency under any other license fails the build until it is reviewed and
  added to the `exceptions` list with justification.
- **Bans / duplicates** — duplicate versions and wildcard version requirements
  are surfaced as warnings; specific crates can be hard-denied via
  `[bans].deny`.
- **Sources** — dependencies may only come from the crates.io registry.
  Unknown registries and arbitrary git sources are rejected.

### Running the checks locally

```bash
# One-time install
cargo install cargo-deny --locked
cargo install cargo-audit --locked

# Full policy check (advisories, bans, licenses, sources)
cargo deny check

# Vulnerability-only scan
cargo audit
```

### Accepting an exception

If an advisory or license genuinely cannot be remediated immediately, add a
scoped entry to the relevant section of [`deny.toml`](deny.toml)
(`[advisories].ignore`, `[licenses].exceptions`, or `[bans].allow`) **with a
comment** describing the reason and a tracking link. Exceptions should be rare
and time-bounded.

## Reporting a vulnerability

Please report security vulnerabilities privately to the maintainers rather than
opening a public issue, so a fix can be prepared before disclosure.
