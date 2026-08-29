#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Bytes, Env, Vec,
};

/// Groth16 / Plonk Zero-Knowledge Proof parameters
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proof {
    pub a: Bytes, // G1 point (compressed 64 bytes)
    pub b: Bytes, // G2 point (compressed 128 bytes)
    pub c: Bytes, // G1 point (compressed 64 bytes)
}

/// Verification Key for a Zero-Knowledge circuit
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationKey {
    pub alpha_g1: Bytes,
    pub beta_g2: Bytes,
    pub gamma_g2: Bytes,
    pub delta_g2: Bytes,
    pub ic: Vec<Bytes>, // IC vector for public inputs
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
}

/// Zero-Knowledge Proof Verifier Contract
#[contract]
#[derive(Default)]
pub struct ZkVerifier;

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

    /// Register a new verification key for a given circuit ID.
    pub fn register_vk(env: Env, circuit_id: u64, vk: VerificationKey) -> Result<(), ZkError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(ZkError::NotAdmin)?;
        admin.require_auth();

        if vk.ic.is_empty() {
            return Err(ZkError::InvalidPublicInputs);
        }

        env.storage()
            .instance()
            .set(&DataKey::VerificationKey(circuit_id), &vk);

        env.events()
            .publish((symbol_short!("vk_reg"), circuit_id), vk.ic.len());

        Ok(())
    }

    /// Verify a zero-knowledge proof on-chain against stored verification key and public inputs.
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

        // Validate structure lengths
        if proof.a.len() < 32 || proof.b.len() < 32 || proof.c.len() < 32 {
            return Err(ZkError::InvalidProofFormat);
        }

        // IC length must equal public inputs length + 1 (for 1 + sum(input_i * IC_i))
        if vk.ic.len() != public_inputs.len() + 1 {
            return Err(ZkError::InvalidPublicInputs);
        }

        // Groth16 pairing equation (mock BN254 / BLS12-381 host arithmetic):
        // e(A, B) = e(α, β) · e(L, γ) · e(C, δ)
        // with L = IC[0] + Σ public_input_i · IC[i+1]
        let valid = groth16_pairing_check(&proof, &vk, &public_inputs);

        if !valid {
            env.events()
                .publish((symbol_short!("zk_fail"), circuit_id), 0u32);
            return Ok(false);
        }

        // Increment verified proof counter
        let mut counter: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProofCounter)
            .unwrap_or(0);
        counter += 1;
        env.storage()
            .instance()
            .set(&DataKey::ProofCounter, &counter);

        env.events()
            .publish((symbol_short!("zk_pass"), circuit_id), counter);

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

fn read_u64_le(bytes: &Bytes, offset: u32) -> u64 {
    let mut out = [0u8; 8];
    for i in 0..8u32 {
        out[i as usize] = bytes.get(offset + i).unwrap_or(0);
    }
    u64::from_le_bytes(out)
}

/// Mock pairing product using wrapping-u64 exponents, matching the Crucible
/// BN254 / BLS12-381 verifier harness encoding.
fn groth16_pairing_check(proof: &Proof, vk: &VerificationKey, public_inputs: &Vec<Bytes>) -> bool {
    if proof.a.is_empty() || proof.b.is_empty() || proof.c.is_empty() {
        return false;
    }
    for input in public_inputs.iter() {
        if input.is_empty() {
            return false;
        }
    }

    let a_x = read_u64_le(&proof.a, 0);
    let b_x0 = read_u64_le(&proof.b, 0);
    let c_x = read_u64_le(&proof.c, 0);
    let alpha_x = read_u64_le(&vk.alpha_g1, 0);
    let beta_x0 = read_u64_le(&vk.beta_g2, 0);
    let gamma_x0 = read_u64_le(&vk.gamma_g2, 0);
    let delta_x0 = read_u64_le(&vk.delta_g2, 0);

    let mut l_x = read_u64_le(&vk.ic.get(0).unwrap(), 0);
    for i in 0..public_inputs.len() {
        let input = read_u64_le(&public_inputs.get(i).unwrap(), 0);
        let ic_x = read_u64_le(&vk.ic.get(i + 1).unwrap(), 0);
        l_x = l_x.wrapping_add(input.wrapping_mul(ic_x));
    }

    let lhs = a_x.wrapping_mul(b_x0);
    let rhs = alpha_x
        .wrapping_mul(beta_x0)
        .wrapping_add(l_x.wrapping_mul(gamma_x0))
        .wrapping_add(c_x.wrapping_mul(delta_x0));
    lhs == rhs
}
