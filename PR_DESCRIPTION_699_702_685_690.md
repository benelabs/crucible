This PR provides comprehensive implementations resolving issues across financial smart contracts reentrancy protection, Zero-Knowledge proof verification, backend internal gRPC communication layer, and zero-downtime database migration strategy.

### Summary of Changes

1. **[CONTRACTS] Reentrancy Guard Implementation (#699)**:
   - Added mutex reentrancy guard (`DataKey::ReentrancyGuard`) to `contracts/treasury` and `contracts/insurance`.
   - Protected financial operations (`withdraw`, `flash_loan`, `file_claim`, `approve_claim`) with lock/unlock guard checks to prevent reentrant contract calls.
   - Added unit test in `contracts/treasury/tests/treasury_test.rs` to verify reentrancy protection.

2. **[CONTRACTS] Zero-Knowledge Proof Verification Logic (#702)**:
   - Created new Soroban smart contract crate `contracts/zk_verifier` with `Proof` and `VerificationKey` data structures for Groth16 / Plonk proofs.
   - Implemented `initialize`, `register_vk`, `verify_proof`, and `get_proof_count` functions.
   - Registered `contracts/zk_verifier` in root `Cargo.toml` workspace members and added test suite in `contracts/zk_verifier/tests/zk_verifier_test.rs`.

3. **[BACKEND] gRPC Internal Communication Layer (#685)**:
   - Added Protobuf definitions in `backend/proto/internal_service.proto` for internal microservice RPC calls.
   - Implemented high-performance Tonic-compatible `InternalGrpcService` and `InternalGrpcClient` in `backend/src/services/grpc.rs`.
   - Wired `grpc` module into `backend/src/services/mod.rs` and added integration tests in `backend/tests/grpc_tests.rs`.

4. **[BACKEND] Zero-Downtime Database Migration Strategy (#690)**:
   - Authored zero-downtime database migration guide in `backend/migrations/ZERO_DOWNTIME_GUIDE.md` detailing the Expand and Contract pattern.
   - Created dual-phase SQL migration files (`20260724000000_zero_downtime_expand.sql` & `20260724000001_zero_downtime_contract.sql`).
   - Created automated migration safety check script `backend/scripts/check_migrations.sh` and GitHub Actions workflow `.github/workflows/migration-check.yml`.
   - Added test in `backend/tests/migration_tests.rs`.

Closes #699
Closes #702
Closes #685
Closes #690
