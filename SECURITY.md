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

### Scope

The following are **in scope** for the vulnerability disclosure program:

- The `backend/` API services, authentication (`/api/v1/auth/*`), and the
  contract compilation / simulation / sandbox services.
- The Soroban smart-contract tooling in `crucible-macros/`, `contracts/`, and
  `libs/` that execute or transform untrusted contract source.
- The deployment automation under `deployments/` and `infra/` (Terraform,
  Ansible, Argo CD) that handles production secrets or cluster access.
- The release pipeline (`.github/workflows/release.yml`) and supply-chain
  configuration (`deny.toml`, `.releaserc.json`).

The following are **out of scope**:

- Issues in dependencies already tracked upstream (report those via
  `cargo audit` / `cargo deny` instead — see above).
- Denial-of-service via the public k6 load-generation harness
  (`tests/load/`) which is intentionally adversarial.
- Social engineering, physical security, and volunteered-rate-limit abuse.

### Response SLAs

| Phase | Target |
| --- | --- |
| **Triage** (acknowledge receipt) | **within 24 hours** |
| Initial severity assessment | within 72 hours |
| Status update on active issues | every 7 days |
| Coordinated disclosure | mutually agreed, typically 90 days after fix |

### Encrypted submission (PGP)

Reports may be submitted encrypted to the security contact key so that
zero-day details are never transmitted in plaintext. The canonical key
fingerprint is published below and is **continuously verified** (see
[Automated PGP key verification](#automated-pgp-key-verification)).

- **Security contact:** security@crucible.example.com
- **PGP fingerprint:** `6E0D 3546 2433 132B 906A  53B5 3903 7CFB D628 9960`
- **PGP key ID:** `0xD6289960`
- **Key server:** `hkps://keys.openpgp.org`

The armored public key is committed at
[`tests/security/security_contact.pub.asc`](tests/security/security_contact.pub.asc)
and its fingerprint is continuously verified by CI.

```text
-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEao7MyBYJKwYBBAHaRw8BAQdAB9d/hl6wHV+YZaGkWUaccqx+k+8tjsq62nHV
7L81V3O0PENydWNpYmxlIFNlY3VyaXR5IERpc2Nsb3N1cmUgPHNlY3VyaXR5QGNy
dWNpYmxlLmV4YW1wbGUuY29tPoiQBBMWCgA4FiEEbg01RiQzEyuQalO1OQN8+9Yom
WAFAmqOzMgCGwMFCwkIBwIGFQoJCAsCBBYCAwECHgECF4AACgkQOQN8+9YomWC1
ZQEA5frRrjLb4YDUuy7JXR0sz+2ivu7j2xjlzzzYTpR8LcgBAOa+KpSwdRz6FcwQ
8OmKgtk26Yb/9l1U+mROEyjhgeMIuDgEao7MyBIKKwYBBAGXVQEFAQEHQAoGTrcR
C2FNdWnBOdyASE7+L4pGOmcoh0ZXccqImkBTAwEIB4h4BBgWCgAgFiEEbg01RiQz
EyuQalO1OQN8+9YomWAFAmqOzMgCGwwACgkQOQN8+9YomWBs3QEAyLYLXQU3CyU0
rsQGJvZm9oTtqmvgQnzG1QPYP2mWG0IBAJhkYBiFD2RKY1+D29hAYuYOAQ+yXqEV
u1pN0xZrFsIJ
=Jfqm
-----END PGP PUBLIC KEY BLOCK-----
```

> **Note:** Keep the fingerprint above in sync with
> `tests/security/security_contact.pub.asc`. The CI job
> `.github/workflows/security-contact-verify.yml` fails the build if the
> fingerprint documented here drifts from the published key.

### Automated PGP key verification

The documented fingerprint is guarded by an automated test:

- `tests/security/verify_pgp_key.sh` — downloads the published key,
  asserts its fingerprint matches the value documented in this file, and
  exits non-zero on mismatch.
- `.github/workflows/security-contact-verify.yml` — runs the verification
  on every push/PR and on a weekly schedule.

### Vulnerability severity tiers

| Tier | Examples | First-response SLA |
| --- | --- | --- |
| **Critical** | RCE in compilation sandbox, auth bypass, private-key leakage | 24h |
| **High** | SQL/NoSQL injection, privilege escalation, unauthenticated data exposure | 24h |
| **Medium** | CSRF, information disclosure, rate-limit bypass | 72h |
| **Low** | Verbose errors leaking stack traces, missing security headers | 7d |

### Bug bounty payout tiers

Payouts are awarded at the discretion of the maintainers for valid,
previously-unknown, in-scope vulnerabilities reported through the private
channel above.

| Severity | Indicative payout (USD) |
| --- | --- |
| **Critical** | $5,000 – $15,000 |
| **High** | $1,500 – $5,000 |
| **Medium** | $300 – $1,500 |
| **Low** | $50 – $300 |

Eligibility rules:

1. The vulnerability must be reported **privately** and not disclosed
   publicly before a fix is released.
2. The reporter must not have exploited the issue beyond what is strictly
   necessary to demonstrate it, and must not have accessed third-party data.
3. Employees, contractors, and individuals with prior written authorization
   are not eligible for monetary rewards.
4. Duplicate reports are rewarded to the first valid submission only.

### Coordinated disclosure

Once a fix lands, we will:

1. Publish a CVE / GitHub Security Advisory.
2. Credit the reporter (unless anonymity is requested).
3. Summarize the impact and remediation in the release notes.
