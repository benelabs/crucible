# Cryptographic Best Practices for ZK Verifier & Treasury

## Overview

This document provides security guidance for implementing the cryptographic fixes across the zk_verifier and treasury contracts. It covers both the mathematics behind the remediations and practical hardening strategies.

---

## Part 1: Field Arithmetic and Scalar Validation

### The Field Modulus Problem

**BN254 Scalar Field Modulus:**
```
r = 52435875175126190479447740508185965837690552500527637822603658699938581184513
  = 0x30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd47
```

**Why It Matters:**
- Zero-knowledge proofs operate in the scalar field Fr (not the base field Fp)
- Public inputs are field elements: they must satisfy 0 ≤ x < r
- Non-canonical inputs (x' = x + r) may not fail proof verification but violate circuit semantics
- Attack vector: Attacker submits x' and x separately, triggering different circuit behaviors

### Constant-Time Comparison

**Incorrect (leaks timing info):**
```rust
// DON'T DO THIS — exits early when first limb differs
if x_limbs[3] > FIELD_MODULUS_LIMBS[3] {
    return Err(InvalidPublicInputs);
}
```

**Correct (constant-time):**
```rust
// Compare all limbs; accumulate borrow without early return
let mut borrow = false;
for i in 0..4 {
    let (diff, new_borrow) = x_limbs[i].overflowing_sub(FIELD_MODULUS_LIMBS[i]);
    borrow = if borrow {
        new_borrow || (diff == 0)
    } else {
        new_borrow
    };
}
!borrow  // No borrow means x < modulus
```

**Why Constant-Time Matters:**
- Side-channel attack: Attacker measures execution time to infer which path was taken
- Micropipeline analysis: Intel/ARM CPUs may leak secret information through timing variations
- Production systems should use libraries like `dalek` or `zk-std` which provide constant-time ops

### Validating Public Inputs

**Full Validation Flow:**
```
1. Check if empty → reject
2. Check if all-zero → reject (point at infinity encoding)
3. Extract as 256-bit big-integer (32 bytes)
4. Compare against r in constant-time
5. If x >= r, reject with InvalidPublicInputs
6. Otherwise, accept
```

**Why this matters in circuit design:**
- If circuit expects x < r but receives x' = x + r, gate outputs may be incorrect
- Example: If circuit computes `y = x^(-1) mod r`, submitting x' may produce y' ≠ y^(-1)
- This breaks proof soundness even if proof verifies

---

## Part 2: Elliptic Curve Point Validation

### BN254 Curve Parameters

```
Base Field (Fp):  p = 21888242871839275222246405745257275088548364400416034343698204186575808495617
Scalar Field (Fr): r = 52435875175126190479447740508185965837690552500527637822603658699938581184513

G1 Curve Equation:  y² = x³ + 3 (mod p)
G2 Extension Field: Fp2 = Fp[i] where i² = -1

G1 Point Length: 64 bytes (compressed)
  - First 32 bytes: X coordinate
  - Next 32 bytes: Y coordinate

G2 Point Length: 128 bytes (compressed)
  - First 32 bytes: X₀ (real part of x)
  - Next 32 bytes: X₁ (imaginary part of x)
  - Next 32 bytes: Y₀ (real part of y)
  - Next 32 bytes: Y₁ (imaginary part of y)
```

### The Partial Validation Vulnerability

**Vulnerable Code:**
```rust
// Only reads first 8 bytes (64 bits) of each 32-byte coordinate
let x = read_u64_le(&bytes, 0);     // Only first 64 bits
let y = read_u64_le(&bytes, 8);     // Only second 64 bits
let y2 = y.wrapping_mul(y);
let x3 = x.wrapping_mul(x).wrapping_mul(x);
if y2 != x3.wrapping_add(3) { 
    return Err(PointNotOnCurve);  // May pass even if full point is invalid!
}
```

**Attack Scenario:**
1. Attacker crafts 64-byte G1 point where:
   - First 8 bytes satisfy y² = x³ + 3 (mod 2^64)
   - Remaining 56 bytes contain arbitrary data
2. Point fails full 256-bit curve check but passes partial validation
3. Invalid point enters pairing computation, potentially corrupting result

**Fix: Full 256-bit Validation**
```rust
// Extract all 4 limbs for each coordinate
let x_limbs = extract_256bit_limbs(&bytes, 0);
let y_limbs = extract_256bit_limbs(&bytes, 32);

// Verify each limb is within Fp
if !is_less_than_field_prime(&x_limbs) || !is_less_than_field_prime(&y_limbs) {
    return Err(PointNotOnCurve);
}

// Verify full 256-bit curve equation
if !verify_weierstrass_equation(&x_limbs, &y_limbs) {
    return Err(PointNotOnCurve);
}
```

### Subgroup Membership

**The Cofactor Issue:**
- BN254 G1 has cofactor 1 (all points satisfy curve equation are in the prime-order subgroup)
- BLS12-381 G1 has cofactor 1 (same property)
- However, BLS12-381 G2 has cofactor 972 (requires explicit cofactor clearing)

**Attack: Small-Order Torsion Points**
1. Attacker submits a point P of order h (cofactor), not order r
2. Point P is on the curve but not in the prime-order subgroup
3. Pairing e(P, Q) may reveal information about Q (discrete log attack)
4. Or: e(P, Q) = 1 for any Q, making verification trivial

**Defense: Reject Small-Order Points**
```rust
fn in_prime_subgroup_full(x: &[u64; 4], y: &[u64; 4]) -> bool {
    // Reject coordinates divisible by small cofactors
    const SMALL_FACTORS: &[u64] = &[3, 5, 7, 11, 13];
    
    let x0 = x[0];
    let y0 = y[0];
    
    for factor in SMALL_FACTORS {
        if x0 % factor == 0 && y0 % factor == 0 {
            return false;  // Likely small-order point
        }
    }
    true
}
```

**Note:** This is a heuristic check. Full subgroup membership requires testing [r]P = O, which is expensive. The heuristic rejects obvious torsion but isn't foolproof—consider delegating to Soroban's native pairing precompiles for production systems.

---

## Part 3: Proof Replay Protection

### Why Nullifiers Are Essential

**Scenario without nullifiers:**
```
1. Off-chain: Compute proof π for statement x with evidence w
2. On-chain: Call verify_proof(π, x) → returns true
3. Attacker: Call verify_proof(π, x) again with same π and x
4. Result: Proof verifies again; any downstream action (e.g., "unlock feature") triggers twice
```

**Risk Examples:**
- Voting contract: Proof used to cast vote in ballot A, then replayed in ballot B
- Withdrawal proof: Same proof used to withdraw twice
- Access control: Same proof used to unlock access multiple times

### Nullifier Design

**Cryptographic Nullifier:**
```
nullifier = H(proof.a || proof.b || proof.c || public_inputs[0] || ... || public_inputs[n-1])

where H = SHA-256
```

**Why hash proof components?**
- Any change to the proof (even 1 bit) produces a completely different nullifier (avalanche effect)
- Attacker cannot forge a proof with a target nullifier (pre-image resistance)
- Nullifier is tied to the exact proof; can't replay equivalent proofs from different sources

**Storage:**
```rust
// Map: nullifier (32 bytes) → consumed (bool)
ProofNullifiers: Map<Bytes, bool>

// After successful verification:
nullifiers.set(nullifier, true);
```

**Lookups:**
```rust
// Before verification:
if nullifiers.get(nullifier).unwrap_or(false) {
    return Ok(false);  // Proof already consumed
}
```

### Lifecycle

```
Time  Event                          Storage State
----  -----                          ---------------
T0    Proof submitted for first time nullifier not in map
T1    verify_proof() succeeds        nullifier → true (inserted)
T2    Same proof submitted again     nullifier exists → rejected immediately
T3    Different proof submitted      different nullifier → allowed if valid
```

### Nullifier Expiration (Optional)

For long-running systems, consider auto-expiring old nullifiers:

```rust
#[contracttype]
pub struct ProofRecord {
    pub consumed: bool,
    pub timestamp: u64,  // Ledger sequence when consumed
}

// Periodically clean up old entries:
if ledger.sequence() > record.timestamp + EXPIRATION_WINDOW {
    nullifiers.remove(key);  // Allow re-verification after expiration
}
```

---

## Part 4: Token Transfer Security

### The Vulnerability

**Current (Broken) Flow:**
```
1. Check quorum (admins authorize) ✓
2. Verify balance available ✓
3. Deduct from internal accounting ✓
4. Emit "withdraw" event ✓
5. Return operation ID
→ BUT: Never call token.transfer()!

Result: Funds stay locked; recipient gets no tokens
```

### Fixed Flow

**Immediate Withdrawal:**
```
1. Check quorum ✓
2. Verify balance ✓
3. Deduct from accounting ✓
4. [NEW] Call token.transfer(...) ✓
5. Emit event
6. Return 0
```

**Queued Withdrawal (execute_withdrawal):**
```
1. Lookup pending operation
2. Verify not cancelled ✓
3. Verify timelock elapsed ✓
4. Remove from pending queue ✓
5. [NEW] Call token.transfer(...) ✓
6. Emit event
7. Return
```

### Token Interface

```rust
use soroban_sdk::token;

// Signature: transfer(from, to, amount)
token::Client::new(&env, &token_address).transfer(
    &from_address,                        // Who sends tokens
    &to_address,                          // Who receives tokens
    &amount,                              // How many tokens (with decimals)
);
```

**Critical Points:**
- `from` must be the treasury contract itself (`env.current_contract_address()`)
- `to` is the recipient (withdrawal destination)
- `amount` is the withdrawal amount (in token base units, including decimals)
- Call must succeed; if it fails, the transaction reverts (accounting remains consistent)

### Error Handling

```rust
// If transfer fails, what happens?
match token::Client::new(&env, &token).transfer(...) {
    Ok(_) => {
        env.events().publish((symbol_short!("success"),), ...);
    }
    Err(e) => {
        // Funds were already deducted from accounting
        // Need recovery mechanism or restore before returning error
        restore_balance(&env, treasury_addr, &token, amount);
        panic_with_error!(&env, ContractError::TransferFailed);
    }
}
```

**Best Practice:** Ensure transfer succeeds before deducting balance (atomicity):

```rust
// Transfer FIRST, then deduct
token::Client::new(&env, &token).transfer(&from, &to, &amount);

// Then update accounting
let mut balances: Map<...> = env.storage().instance().get(...).unwrap();
balances.set(key, current - amount);
env.storage().instance().set(&DataKey::Balances, &balances);
```

---

## Part 5: Hardening Strategies

### 1. Defensive Programming

**Input Validation:**
```rust
// Before any computation, validate ALL inputs
pub fn withdraw(..., signers: Vec<Address>) -> u64 {
    // Check 1: Are signers present?
    if signers.is_empty() {
        panic_with_error!(&env, ContractError::InsufficientQuorum);
    }
    
    // Check 2: Is amount positive?
    if amount <= 0 {
        panic_with_error!(&env, ContractError::InvalidAmount);
    }
    
    // Check 3: Is recipient valid (not zero address)?
    if to == Address::from([0; 32]) {
        panic_with_error!(&env, ContractError::InvalidRecipient);
    }
    
    // Only then proceed
    ...
}
```

### 2. State Machine Clarity

**Track state explicitly:**
```rust
#[contracttype]
enum WithdrawalState {
    Pending,
    Timelocked,
    Executed,
    Cancelled,
}

// Verify state transitions are valid
if state == WithdrawalState::Executed {
    panic!("Cannot re-execute completed withdrawal");
}
```

### 3. Auditing

**Emit events for all critical operations:**
```rust
env.events().publish(
    (symbol_short!("proof_verified"), circuit_id),
    (
        proof_hash,
        signer,
        timestamp,
        proof_counter,
    )
);
```

**Off-chain monitors** can:
- Track proof success rate
- Alert on unusual patterns (e.g., many failed verifications)
- Detect replay attempts
- Monitor token transfer success

### 4. Reentrancy Guards

**Already implemented in treasury; verify in zk_verifier:**
```rust
fn lock_guard(env: &Env) {
    if env.storage().instance().get::<_, bool>(&DataKey::ReentrancyGuard).unwrap_or(false) {
        panic_with_error!(env, ContractError::ReentrancyGuardLocked);
    }
    env.storage().instance().set(&DataKey::ReentrancyGuard, &true);
}

fn unlock_guard(env: &Env) {
    env.storage().instance().set(&DataKey::ReentrancyGuard, &false);
}
```

### 5. Gas & Complexity Limits

**Prevent DoS through unbounded loops:**
```rust
const MAX_PUBLIC_INPUTS: u32 = 32;
if public_inputs.len() > MAX_PUBLIC_INPUTS {
    return Err(ZkError::InvalidPublicInputs);  // Cap circuit complexity
}
```

---

## Part 6: Testing Strategy

### Unit Tests (Crypto)

```rust
#[test]
fn test_non_canonical_scalar_rejected() {
    let mut scalar = [0u8; 32];
    // Set scalar = r + 1 (non-canonical)
    let r_plus_1 = FIELD_MODULUS + 1;
    write_limbs(&mut scalar, r_plus_1);
    
    assert!(validate_public_input_scalar(&Bytes::from_array(scalar)).is_err());
}

#[test]
fn test_off_curve_point_rejected() {
    let point = craft_off_curve_point();  // y² ≠ x³ + 3
    assert!(validate_g1_point(&point).is_err());
}

#[test]
fn test_proof_replay_rejected() {
    let result1 = verify_proof(...);
    assert!(result1.is_ok());
    
    let result2 = verify_proof(...);  // Same proof
    assert!(result2 == Ok(false));  // Replay detected
}
```

### Integration Tests

```rust
#[test]
fn test_treasury_withdrawal_end_to_end() {
    let env = Env::default();
    initialize_treasury(&env, ...);
    deposit_tokens(&env, 1000);
    
    let receipt = withdraw(&env, recipient, 500, signers);
    
    // Verify recipient balance increased
    assert_eq!(get_recipient_balance(&env, recipient), 500);
    
    // Verify treasury balance decreased
    assert_eq!(get_treasury_balance(&env), 500);
}
```

### Fuzzing

```rust
#[test]
fn fuzz_public_inputs(scalar_bytes: [u8; 32]) {
    let bytes = Bytes::from_array(scalar_bytes);
    
    // Should never panic
    let _ = validate_public_input_scalar(&bytes);
    
    // If accepted, scalar must be < r
    if let Ok(_) = validate_public_input_scalar(&bytes) {
        let as_int = bytes_to_biguint(&bytes);
        assert!(as_int < FIELD_MODULUS);
    }
}
```

---

## References & Further Reading

1. **BN254 Specification**: https://github.com/ethereum/py_pairing (EIP-197)
2. **BLS12-381**: https://github.com/zkcrypto/bls12_381
3. **Constant-Time Arithmetic**: https://cr.yp.to/constant-time.html
4. **Soroban SDK Docs**: https://docs.rs/soroban-sdk/
5. **ZK Proof Best Practices**: https://www.zeroknowledgeblog.com/

