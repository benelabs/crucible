# Security Remediation Implementation Checklist

## Pre-Implementation Review

- [ ] **Code Audit**: Review both remediation.md and existing contract code line-by-line
- [ ] **Dependency Check**: Verify soroban_sdk version supports `sha256()` for nullifier computation
- [ ] **Test Coverage**: Ensure test suite exists; add tests for each remediation
- [ ] **Backup**: Commit current code state before applying changes

---

## Vulnerability #1: Non-Canonical Scalar Validation

### Implementation Steps

- [ ] **Define Field Modulus**: Add `FIELD_MODULUS_LIMBS` constant (BN254 r or BLS12-381 r)
- [ ] **Add is_less_than_modulus()**: Implement constant-time comparison function
- [ ] **Replace validate_public_input_scalar()**: 
  - [ ] Remove wrapping-arithmetic checks
  - [ ] Add full 256-bit limb extraction
  - [ ] Call `is_less_than_modulus()` for canonical range verification
- [ ] **Add Error Code**: Ensure `InvalidPublicInputs` error is emitted on failure

### Testing

- [ ] **Unit Test: Zero value**: Reject scalar = 0
- [ ] **Unit Test: Canonical range**: Accept scalars 0 < s < r
- [ ] **Unit Test: Non-canonical high**: Reject s >= r
- [ ] **Unit Test: Max uint256**: Reject 2^256 - 1
- [ ] **Fuzz Test**: Generate random 256-bit values, verify rejection for s >= r
- [ ] **Property Test**: For any accepted scalar, verify s < r via independent calculation

---

## Vulnerability #2: Proof Replay Attacks

### Implementation Steps

- [ ] **Add DataKey Variant**: Add `ProofNullifiers` to enum
- [ ] **Implement compute_proof_nullifier()**:
  - [ ] Concatenate proof.a + proof.b + proof.c + public_inputs
  - [ ] Hash with `soroban_sdk::crypto::sha256()`
  - [ ] Return 32-byte Bytes
- [ ] **Initialize Map**: Add initialization of nullifier map in contract setup
- [ ] **Update verify_proof()**:
  - [ ] Compute nullifier before validation
  - [ ] Check nullifier membership (return false if already consumed)
  - [ ] Store nullifier after successful verification
  - [ ] Emit `zk_replay` event on replay attempt
- [ ] **Add Error Code**: Ensure nullifier-related errors are tracked

### Testing

- [ ] **Unit Test: Fresh proof**: First verification succeeds, increments counter
- [ ] **Unit Test: Replay detection**: Submitting same proof twice rejects second attempt
- [ ] **Unit Test: Proof mutation**: Changing one byte of proof changes nullifier (different hash)
- [ ] **Unit Test: Different inputs**: Same proof + different public_input produces different nullifier
- [ ] **Integration Test**: Two independent proofs (different nullifiers) both succeed
- [ ] **Fuzz Test**: Generate random proof combinations; verify nullifier uniqueness

---

## Vulnerability #3: Missing Token Transfer

### Implementation Steps

- [ ] **Immediate Withdrawal Path**:
  - [ ] Add call to `token::Client::new(&env, &token).transfer(...)`
  - [ ] Transfer from `env.current_contract_address()` to `to`
  - [ ] Execute BEFORE emitting withdrawal event
- [ ] **Queued Withdrawal Path** (execute_withdrawal):
  - [ ] Add same `token::Client::transfer()` call
  - [ ] Execute AFTER timelock check, BEFORE event emission
  - [ ] Handle token client errors gracefully (don't leave accounting inconsistent)
- [ ] **Error Recovery**: Ensure failed transfers don't leave funds reserved but untransferred
  - [ ] Consider implementing a "recovery" mechanism or emergency pause

### Testing

- [ ] **Unit Test: Immediate transfer**: Small withdrawal completes in same tx, balance decreases
- [ ] **Unit Test: Queued transfer**: After timelock elapses, balance leaves contract
- [ ] **Integration Test: Token mock**: Mock token client, verify transfer called with correct args
- [ ] **Integration Test: Insufficient balance**: Transfer reverts if contract balance < amount
- [ ] **End-to-End Test**: 
  - [ ] Deposit tokens into treasury
  - [ ] Withdraw with proper signatures
  - [ ] Verify recipient account balance increased
  - [ ] Verify treasury balance decreased
- [ ] **Stress Test**: Multiple concurrent withdrawals don't race or deadlock

---

## Vulnerability #4: Incomplete Curve Point Validation

### Implementation Steps

- [ ] **Add FIELD_PRIME_LIMBS**: Define 256-bit field modulus constant
- [ ] **Implement extract_256bit_limbs()**: Extract 4 x 64-bit limbs from 32 bytes at offset
- [ ] **Implement is_less_than_field_prime()**: Constant-time comparison against prime
- [ ] **Implement verify_weierstrass_equation()**: Validate Y² = X³ + b (mod p)
- [ ] **Implement in_prime_subgroup_full()**: Check all coordinates against small cofactors
- [ ] **Replace validate_g1_point()**:
  - [ ] Use full 256-bit validation
  - [ ] Call `extract_256bit_limbs()` for both X and Y
  - [ ] Verify both coordinates < p
  - [ ] Verify curve equation
  - [ ] Verify subgroup membership
- [ ] **Replace validate_g2_point()** with G2-specific version:
  - [ ] Extract both Fp2 components (x0, x1, y0, y1)
  - [ ] Verify all four 256-bit values < p
  - [ ] Verify G2 curve equation
  - [ ] Verify cofactor-cleared subgroup membership
- [ ] **Update groth16_pairing_check()**:
  - [ ] Use full-precision limb arithmetic
  - [ ] Validate all points before pairing equation check
  - [ ] Compare full 256-bit coordinates (not just first 8 bytes)

### Testing

- [ ] **Unit Test: Identity rejection**: All-zero bytes rejected (point at infinity)
- [ ] **Unit Test: Partial validation bypass**: Point with valid first 8 bytes but invalid remainder rejected
- [ ] **Unit Test: Out-of-field X**: X >= p rejected
- [ ] **Unit Test: Out-of-field Y**: Y >= p rejected
- [ ] **Unit Test: Off-curve point**: Point not satisfying Y² = X³ + 3 rejected
- [ ] **Unit Test: Small-order torsion**: Points with small cofactor rejected
- [ ] **Unit Test: Valid point**: Legitimate on-curve, in-subgroup point accepted
- [ ] **Fuzz Test: Malicious G1**: Generate 64 bytes with valid first 8 but invalid rest; verify rejection
- [ ] **Fuzz Test: Malicious G2**: Generate 128 bytes with crafted byte patterns; verify validation
- [ ] **Property Test**: For any accepted point, independently verify curve equation

---

## Cross-Cutting Concerns

### Constant-Time Arithmetic

- [ ] All comparison and validation functions use constant-time operations
- [ ] No early returns that leak timing information about scalar/point values
- [ ] Use wrapping arithmetic consistently for mock calculations

### Error Handling

- [ ] All validation failures emit descriptive events for auditing
- [ ] Error codes are unique and logged
- [ ] No stack overflows from recursive or unbounded loops
- [ ] Bounds checks on all vector/array accesses

### Documentation

- [ ] Add inline comments explaining curve parameters (modulus, b coefficient)
- [ ] Document constant-time algorithms and why they're necessary
- [ ] Include references to BN254 / BLS12-381 specifications
- [ ] Note any divergences from standard implementations

---

## Integration Testing

### End-to-End Scenarios

**Scenario 1: Legitimate ZK Workflow**
- [ ] Deploy verifier with test circuit
- [ ] Register verification key
- [ ] Generate valid proof off-chain
- [ ] Submit proof on-chain → verification succeeds
- [ ] Query proof counter → incremented

**Scenario 2: Replay Attack Prevented**
- [ ] Submit same proof again → verification fails / returns false
- [ ] Verify event shows `zk_replay` or similar

**Scenario 3: Non-Canonical Scalar Rejection**
- [ ] Create proof with public_input = r (field modulus)
- [ ] Submission fails with `InvalidPublicInputs` error

**Scenario 4: Malicious Point Encoding**
- [ ] Craft G1 point with valid first 8 bytes but rest all 0xFF
- [ ] Submission fails with `PointNotOnCurve` or `PointNotInSubgroup`

**Scenario 5: Treasury Withdrawal End-to-End**
- [ ] Deposit 1000 tokens
- [ ] Request withdrawal of 500 tokens (below threshold)
- [ ] Verify recipient receives 500 tokens immediately
- [ ] Verify treasury balance = 500

**Scenario 6: Queued Withdrawal Execution**
- [ ] Deposit 100,000 tokens
- [ ] Request withdrawal of 50,000 tokens (above threshold)
- [ ] Withdrawal queued with op_id returned
- [ ] Wait for timelock to elapse
- [ ] Call execute_withdrawal(op_id)
- [ ] Verify recipient receives 50,000 tokens

---

## Performance Considerations

- [ ] Full 256-bit validation adds ~10-20% gas overhead (expected)
- [ ] Nullifier storage map grows linearly with proofs; consider expiration policy
- [ ] Token transfer calls add per-transaction cost (~5-10% per withdrawal)
- [ ] All constant-time operations should not cause timeouts

---

## Deployment Checklist

- [ ] All tests pass locally
- [ ] Testnet deployment successful
- [ ] Security audit completed (formal or peer review)
- [ ] Staging environment validation
- [ ] Rollback plan documented (in case of critical issues)
- [ ] Monitoring/alerting set up for abnormal verification patterns
- [ ] Communication to stakeholders about new constraints

---

## Post-Deployment Monitoring

- [ ] Monitor `zk_replay` events for attack attempts
- [ ] Track `InvalidPublicInputs` errors (shouldn't exceed baseline)
- [ ] Verify token transfer success rates approach 100%
- [ ] Monitor nullifier map growth rate
- [ ] Alert on unexpected error codes / validation failures

