#![no_std]

//! Groth16 proof verification on BN254, using Soroban's audited host pairing
//! primitives. Proof points are Ethereum-compatible uncompressed encodings:
//! G1 is `be(X) || be(Y)` (64 bytes), G2 is
//! `be(X.c1) || be(X.c0) || be(Y.c1) || be(Y.c0)` (128 bytes), and public
//! inputs are canonical big-endian BN254 scalar-field elements (32 bytes).
//!
//! The verification equation is
//! `e(A, B) = e(α, β) · e(L, γ) · e(C, δ)` with
//! `L = IC[0] + Σ input_i · IC[i+1]`, checked as a single multi-pairing:
//! `e(A, B) · e(-α, β) · e(-L, γ) · e(-C, δ) = 1`.

use core::convert::TryInto;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype,
    crypto::{
        bn254::{Bn254Fr, Bn254G1Affine, Bn254G2Affine},
        Hash,
    },
    panic_with_error, vec, Address, Bytes, BytesN, Env, Vec, U256,
};

/// Groth16 proof over BN254.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proof {
    /// Uncompressed G1 point, 64 bytes.
    pub a: Bytes,
    /// Uncompressed G2 point, 128 bytes.
    pub b: Bytes,
    /// Uncompressed G1 point, 64 bytes.
    pub c: Bytes,
}

/// Verification key for a Groth16 circuit over BN254.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationKey {
    pub alpha_g1: Bytes,
    pub beta_g2: Bytes,
    pub gamma_g2: Bytes,
    pub delta_g2: Bytes,
    pub ic: Vec<Bytes>, // IC vector: one point per public input, plus IC[0]
}

#[contracttype]
enum DataKey {
    Admin,
    VerificationKey(u64), // circuit_id -> VerificationKey
    ProofCounter,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ZkError {
    NotAdmin = 1,
    CircuitNotFound = 2,
    InvalidProofFormat = 3,
    InvalidPublicInputs = 4,
    VerificationFailed = 5,
    AlreadyInitialized = 6,
    /// Point is the curve identity (infinity) — rejected before pairing.
    PointAtInfinity = 7,
    /// Coordinates do not satisfy the BN254 curve equation.
    PointNotOnCurve = 8,
    /// Reserved for future use by a verifier with a non-cofactor-free group.
    PointNotInSubgroup = 9,
    /// Public input is not a canonical BN254 scalar (`>= r`).
    InvalidScalar = 10,
}

/// Uncompressed BN254 G1 point length.
const G1_LEN: usize = 64;
/// Uncompressed BN254 G2 point length.
const G2_LEN: usize = 128;
/// BN254 scalar-field element length.
const SCALAR_LEN: usize = 32;
/// Maximum number of public inputs accepted by a single proof.
const MAX_PUBLIC_INPUTS: u32 = 32;
/// Domain separator for the public-input commitment hash.
const INPUT_DOMAIN: &[u8; 21] = b"crucible.zk.inputs.v1";

/// BN254 base-field modulus p, big-endian (matches soroban-sdk's bn254 module).
const BN254_FP_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
];

/// BN254 scalar-field modulus r, big-endian (matches soroban-sdk's bn254 module).
const BN254_FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// Zero-Knowledge Proof Verifier Contract (BN254 / Groth16).
#[contract]
#[derive(Default)]
pub struct ZkVerifier;

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VkRegistered {
    #[topic]
    pub circuit_id: u64,
    pub ic_len: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofRejected {
    #[topic]
    pub circuit_id: u64,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofVerified {
    #[topic]
    pub circuit_id: u64,
    pub count: u64,
}

/// Domain-separated SHA-256 commitment over the public inputs of a verified
/// proof. This is emitted for callers that need to bind a proof to its
/// statement; it is never used as a substitute for pairing verification.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicInputsCommitted {
    #[topic]
    pub circuit_id: u64,
    pub digest: Bytes,
}

#[contractimpl]
impl ZkVerifier {
    /// Initialize the ZK verifier contract with an admin address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, ZkError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::ProofCounter, &0u64);
    }

    /// Register a Groth16 verification key for a circuit ID. Admin only.
    pub fn register_vk(env: Env, circuit_id: u64, vk: VerificationKey) -> Result<(), ZkError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ZkError::NotAdmin)?;
        admin.require_auth();

        if vk.ic.is_empty() || vk.ic.len() > MAX_PUBLIC_INPUTS + 1 {
            return Err(ZkError::InvalidPublicInputs);
        }

        // Reject malformed VK points before they can reach a pairing call.
        validate_g1(&env, &vk.alpha_g1)?;
        validate_g2(&vk.beta_g2)?;
        validate_g2(&vk.gamma_g2)?;
        validate_g2(&vk.delta_g2)?;
        for ic in vk.ic.iter() {
            validate_g1(&env, &ic)?;
        }

        env.storage()
            .instance()
            .set(&DataKey::VerificationKey(circuit_id), &vk);

        VkRegistered {
            circuit_id,
            ic_len: vk.ic.len(),
        }
        .publish(&env);

        Ok(())
    }

    /// Verify a Groth16 proof against a stored verification key and public inputs.
    ///
    /// Returns `Ok(false)` when the proof is well-formed but the pairing
    /// equation does not hold. Malformed encodings return an error instead.
    pub fn verify_proof(
        env: Env,
        circuit_id: u64,
        proof: Proof,
        public_inputs: Vec<Bytes>,
    ) -> Result<bool, ZkError> {
        let vk: VerificationKey = env
            .storage()
            .instance()
            .get(&DataKey::VerificationKey(circuit_id))
            .ok_or(ZkError::CircuitNotFound)?;

        if public_inputs.len() > MAX_PUBLIC_INPUTS || vk.ic.len() != public_inputs.len() + 1 {
            return Err(ZkError::InvalidPublicInputs);
        }

        let a = to_g1(&env, &proof.a)?;
        let b = to_g2(&env, &proof.b)?;
        let c = to_g1(&env, &proof.c)?;
        let alpha = to_g1(&env, &vk.alpha_g1)?;
        let beta = to_g2(&env, &vk.beta_g2)?;
        let gamma = to_g2(&env, &vk.gamma_g2)?;
        let delta = to_g2(&env, &vk.delta_g2)?;

        // L = IC[0] + Σ input_i · IC[i+1], computed with host MSM so all
        // arithmetic is the real BN254 group law (no wrapping-u64 shortcuts).
        let mut points = Vec::new(&env);
        let mut scalars = Vec::new(&env);
        points.push_back(to_g1(&env, &vk.ic.get(0).unwrap())?);
        scalars.push_back(Bn254Fr::from_u256(U256::from_u32(&env, 1)));
        for i in 0..public_inputs.len() {
            points.push_back(to_g1(&env, &vk.ic.get(i + 1).unwrap())?);
            scalars.push_back(to_scalar(&env, &public_inputs.get(i).unwrap())?);
        }

        let bn = env.crypto().bn254();
        let l = bn.g1_msm(points, scalars);

        // e(A,B) · e(-α,β) · e(-L,γ) · e(-C,δ) == 1. The host validates every
        // G2 curve point (on-curve and prime-order subgroup) during this call.
        let valid = bn.pairing_check(
            vec![&env, a, -alpha, -l, -c],
            vec![&env, b, beta, gamma, delta],
        );

        if !valid {
            ProofRejected { circuit_id }.publish(&env);
            return Ok(false);
        }

        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProofCounter)
            .unwrap_or(0);
        counter += 1;
        env.storage()
            .instance()
            .set(&DataKey::ProofCounter, &counter);

        ProofVerified {
            circuit_id,
            count: counter,
        }
        .publish(&env);

        let digest = public_inputs_commitment(&env, circuit_id, &public_inputs);
        PublicInputsCommitted {
            circuit_id,
            digest: Bytes::from_array(&env, &digest.to_array()),
        }
        .publish(&env);

        Ok(true)
    }

    /// Query total verified proofs count.
    pub fn get_proof_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::ProofCounter)
            .unwrap_or(0)
    }
}

/// SHA-256(domain || circuit_id_be || input_count_be || inputs...).
///
/// Domain separation prevents a commitment from being reinterpreted as a hash
/// produced by another protocol. This is a real cryptographic hash; it is used
/// only for the emitted public-input binding, never to "verify" a proof.
fn public_inputs_commitment(env: &Env, circuit_id: u64, public_inputs: &Vec<Bytes>) -> Hash<32> {
    let mut material = Bytes::new(env);
    material.extend_from_array(INPUT_DOMAIN);
    material.extend_from_array(&circuit_id.to_be_bytes());
    material.extend_from_array(&public_inputs.len().to_be_bytes());
    for input in public_inputs.iter() {
        material.append(&input);
    }
    env.crypto().sha256(&material)
}

/// Copies exactly `N` bytes or rejects the encoding.
fn read_exact<const N: usize>(bytes: &Bytes, error: ZkError) -> Result<[u8; N], ZkError> {
    if bytes.len() != N as u32 {
        return Err(error);
    }
    let mut out = [0u8; N];
    bytes.copy_into_slice(&mut out);
    Ok(out)
}

/// Lexicographic big-endian comparison: `a < b`.
fn be_lt(a: &[u8], b: &[u8; 32]) -> bool {
    for i in 0..32 {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
}

/// Canonical BN254 field element: the two high flag bits are unset and the
/// value is strictly below the modulus.
fn valid_field_element(chunk: &[u8]) -> bool {
    chunk[0] & 0xc0 == 0 && be_lt(chunk, &BN254_FP_MODULUS_BE)
}

fn is_zero(raw: &[u8]) -> bool {
    raw.iter().all(|b| *b == 0)
}

fn to_g1(env: &Env, bytes: &Bytes) -> Result<Bn254G1Affine, ZkError> {
    validate_g1(env, bytes)?;
    let raw = read_exact::<G1_LEN>(bytes, ZkError::InvalidProofFormat)?;
    Ok(Bn254G1Affine::from_bytes(BytesN::from_array(env, &raw)))
}

fn to_g2(env: &Env, bytes: &Bytes) -> Result<Bn254G2Affine, ZkError> {
    validate_g2(bytes)?;
    let raw = read_exact::<G2_LEN>(bytes, ZkError::InvalidProofFormat)?;
    Ok(Bn254G2Affine::from_bytes(BytesN::from_array(env, &raw)))
}

fn to_scalar(env: &Env, bytes: &Bytes) -> Result<Bn254Fr, ZkError> {
    let raw = read_exact::<SCALAR_LEN>(bytes, ZkError::InvalidPublicInputs)?;
    if !be_lt(&raw, &BN254_FR_MODULUS_BE) {
        return Err(ZkError::InvalidScalar);
    }
    Ok(Bn254Fr::from_bytes(BytesN::from_array(env, &raw)))
}

/// Validate an uncompressed G1 encoding and its curve membership.
fn validate_g1(env: &Env, bytes: &Bytes) -> Result<(), ZkError> {
    let raw = read_exact::<G1_LEN>(bytes, ZkError::InvalidProofFormat)?;
    if is_zero(&raw) {
        return Err(ZkError::PointAtInfinity);
    }
    let x: [u8; 32] = raw[..32].try_into().unwrap();
    let y: [u8; 32] = raw[32..].try_into().unwrap();
    if !valid_field_element(&x) || !valid_field_element(&y) {
        return Err(ZkError::InvalidProofFormat);
    }
    let point = Bn254G1Affine::from_bytes(BytesN::from_array(env, &raw));
    if !env.crypto().bn254().g1_is_on_curve(&point) {
        return Err(ZkError::PointNotOnCurve);
    }
    Ok(())
}

/// Validate an uncompressed G2 encoding. Full on-curve and subgroup checks are
/// performed by the Soroban host when the point enters `pairing_check`; that
/// is fail-closed (the host traps on invalid G2 points). Rejecting obvious bad
/// encodings here keeps normal failures as typed errors.
fn validate_g2(bytes: &Bytes) -> Result<(), ZkError> {
    let raw = read_exact::<G2_LEN>(bytes, ZkError::InvalidProofFormat)?;
    if is_zero(&raw) {
        return Err(ZkError::PointAtInfinity);
    }
    for chunk in raw.chunks(32) {
        if !valid_field_element(chunk) {
            return Err(ZkError::InvalidProofFormat);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256("crucible.zk.inputs.v1" || u64BE(101) || u32BE(1) || scalar(5)).
    #[test]
    fn public_inputs_commitment_matches_known_sha256_vector() {
        let env = Env::default();
        let mut input = [0u8; SCALAR_LEN];
        input[SCALAR_LEN - 1] = 5;
        let mut inputs = Vec::new(&env);
        inputs.push_back(Bytes::from_array(&env, &input));

        let digest = public_inputs_commitment(&env, 101, &inputs).to_array();
        assert_eq!(
            digest,
            [
                0x0e, 0xf4, 0xa7, 0x34, 0xff, 0xc1, 0x1b, 0x06, 0xf7, 0x5f, 0x21, 0x5e, 0x4c, 0x6b,
                0x8e, 0x2d, 0x47, 0xf7, 0xfc, 0x98, 0x90, 0x7f, 0x37, 0xb3, 0x3f, 0x7c, 0x3e, 0x3a,
                0x8c, 0x39, 0x4a, 0xc2,
            ]
        );
    }
}
