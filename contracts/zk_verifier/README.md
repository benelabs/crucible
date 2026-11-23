# zk_verifier

Groth16 proof verification for Soroban, implemented on **BN254** with the
audited pairing primitives exposed by `soroban_sdk::crypto::bn254`. There is no
mock, stub, or wrapping-integer verification path in this crate. The separate
`crucible::zk` module is a host-only test harness and is not deployable WASM.

## Verification

The contract checks the standard Groth16 equation:

```text
e(A, B) = e(α, β) · e(L, γ) · e(C, δ)
L = IC[0] + Σ input_i · IC[i+1]
```

It is evaluated as a single host multi-pairing:
`e(A, B) · e(-α, β) · e(-L, γ) · e(-C, δ) = 1`. The linear combination `L` is
computed with host multi-scalar multiplication over BN254 G1. The host also
enforces G2 on-curve and prime-order subgroup membership during the pairing
call; malformed G2 encodings therefore fail closed.

### Encodings (exact lengths enforced)

| Value | Encoding | Length |
| --- | --- | --- |
| G1 (`α`, `IC[i]`, `A`, `C`) | `be(X) || be(Y)` | 64 bytes |
| G2 (`β`, `γ`, `δ`, `B`) | `be(X.c1) || be(X.c0) || be(Y.c1) || be(Y.c0)` | 128 bytes |
| Public input | canonical big-endian BN254 `Fr` element (`< r`) | 32 bytes |

The point at infinity, non-canonical field elements, wrong lengths, off-curve
G1 points, and non-canonical scalars are rejected with a typed `ZkError`.
Up to 32 public inputs are accepted, with `ic.len() == inputs.len() + 1`.

A well-formed proof that fails the pairing equation returns `Ok(false)`; it is
never counted as verified. A domain-separated SHA-256 commitment over
`"crucible.zk.inputs.v1" || circuit_id || input_count || inputs` is emitted on
success (`PublicInputsCommitted`) so integrators can bind a proof to its
statement. The hash is an auxiliary binding, not a substitute for verification.

## Public API

- `initialize(admin)` — one-time admin setup
- `register_vk(circuit_id, vk)` — admin-only verification key registration
- `verify_proof(circuit_id, proof, public_inputs) -> Result<bool, ZkError>`
- `get_proof_count() -> u64`

Events: `VkRegistered`, `ProofRejected`, `ProofVerified`,
`PublicInputsCommitted`. Storage keys (`Admin`, `VerificationKey(circuit_id)`,
`ProofCounter`) and their value types are unchanged from the previous release;
no storage migration is required.

## Tests

```sh
cargo test -p zk_verifier --all-features
cargo clippy -p zk_verifier --all-targets -- -D warnings
```

Integration tests use a real BN254 Groth16 proof vector generated with
arkworks (valid on-curve points and canonical scalars), plus tampered proof,
wrong-input, malformed-length, infinity, off-curve, and non-canonical-scalar
negative cases. An inline unit test pins the domain-separated SHA-256 digest
to a fixed expected value.

Pairing checks are significantly more expensive than the previous mock
arithmetic; integrators should budget accordingly and measure with
`cargo test -p zk_verifier -- --nocapture` / Soroban cost diagnostics before
deployment.
