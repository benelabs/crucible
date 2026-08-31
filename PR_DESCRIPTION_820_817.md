This PR resolves issue #820 and issue #817 for `crucible`.

### Summary of Changes

1. **Issue #820 (Cryptographic Signature & Curve Verification Mock Registry)**:
   - Added `CryptoCurve` enum (`Ed25519`, `Secp256k1`, `Secp256r1`).
   - Implemented `MockKeyPair` providing keypair generation, public key conversion to Soroban types (`BytesN<32>`, `BytesN<64>`, `Bytes`), and signature synthesis (`sign`, `sign_bytes`).
   - Added corrupt signature injection (`corrupt_signature`, `corrupt_signature_bytes`) for negative path verification testing.
   - Added `MockCryptoRegistry` for named keypair management and pre-built lookup.
   - Exposed `crypto_registry()` and keypair generator helpers on `MockEnv`.
   - Created test suite `contracts/crucible/src/env_crypto_tests.rs` covering Ed25519, Secp256k1, Secp256r1 signature verification and smart wallet auth flows.

2. **Issue #817 (Event Topic Schema & Filter Subscription Simulator)**:
   - Enhanced `event_topic_match.rs` with wildcard symbol matching (`symbol_short!("*")`, `symbol_short!("_")`, and `Val::VOID`).
   - Added `matches_topic_pattern`, `matches_topics`, `topic_as`, and `assert_schema` methods to `CapturedEvent`.
   - Added `filter_by_topic_pattern` method to `EventMatches`.
   - Implemented `assert_event_matches!` macro with schema assertion capabilities and wildcard topic matching.
   - Enhanced `env_event_filter_tests.rs` with tests matching standard NEP/SEP token event schemas (`transfer`, `mint`, `burn`).

Closes #820
Closes #817
