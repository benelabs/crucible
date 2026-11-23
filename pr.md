# PR: feat(contracts): harden release profiles, SDK policy, and token amount validation

Branch: `fix/prod-ready-083-088-090-092`

Closes #1461
Closes #1466
Closes #1468
Closes #1470

## Summary

- #1461: `deny.toml` bans duplicates/wildcards, release profiles enable overflow checks, three contracts moved to workspace SDK 28, added SDK consistency check script.
- #1466: `from_xlm_str` rejects non-ASCII/empty fraction/>7 decimals, `xlm_frac` rejects non-finite/out-of-range floats, `with_token("XLM", n)` validates n == 7.
- #1468: `MockToken` queries SAC on-chain decimals/admin/symbol, `set_admin` updates cache, tests assert SAC parity.
- #1470: `from_display_amount` rejects malformed inputs, auth/clawback tests assert real behavior, `fund_token` rejects negatives.

## How to test

```
cargo test -p crucible --lib
cargo check -p governance -p insurance -p oracle
bash scripts/check-contract-sdk-version.sh
```

All 276 crucible unit tests pass; all three contracts compile with SDK 28.
