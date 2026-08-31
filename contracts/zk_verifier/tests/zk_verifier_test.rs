#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, Vec};
use zk_verifier::{Proof, VerificationKey, ZkVerifier, ZkVerifierClient};

fn setup_verifier(env: &Env) -> (Address, Address, ZkVerifierClient) {
    let admin = Address::generate(env);
    let contract_id = env.register(ZkVerifier, ());
    let client = ZkVerifierClient::new(env, &contract_id);
    client.initialize(&admin);
    (contract_id, admin, client)
}

fn g1_bytes(env: &Env, x: u64, y: u64) -> Bytes {
    let mut raw = [0u8; 64];
    raw[0..8].copy_from_slice(&x.to_le_bytes());
    raw[8..16].copy_from_slice(&y.to_le_bytes());
    let mut bytes = Bytes::new(env);
    bytes.extend_from_array(&raw);
    bytes
}

fn g2_bytes(env: &Env, x0: u64) -> Bytes {
    let mut raw = [0u8; 128];
    raw[0..8].copy_from_slice(&x0.to_le_bytes());
    let mut bytes = Bytes::new(env);
    bytes.extend_from_array(&raw);
    bytes
}

fn scalar_bytes(env: &Env, scalar: u64) -> Bytes {
    let mut raw = [0u8; 32];
    raw[0..8].copy_from_slice(&scalar.to_le_bytes());
    let mut bytes = Bytes::new(env);
    bytes.extend_from_array(&raw);
    bytes
}

/// Build a Groth16 proof that satisfies e(A,B) = e(α,β)·e(L,γ)·e(C,δ)
/// with A.x = 1 so B.x0 = rhs.
fn valid_groth16(env: &Env, public_input: u64) -> (VerificationKey, Proof, Vec<Bytes>) {
    let alpha_x = 3u64;
    let beta_x0 = 5u64;
    let gamma_x0 = 17u64;
    let delta_x0 = 19u64;
    let ic0_x = 11u64;
    let ic1_x = 13u64;
    let a_x = 1u64;
    let c_x = 7u64;
    let l_x = ic0_x.wrapping_add(public_input.wrapping_mul(ic1_x));
    let b_x0 = alpha_x
        .wrapping_mul(beta_x0)
        .wrapping_add(l_x.wrapping_mul(gamma_x0))
        .wrapping_add(c_x.wrapping_mul(delta_x0));

    let mut ic = Vec::new(env);
    ic.push_back(g1_bytes(env, ic0_x, 1));
    ic.push_back(g1_bytes(env, ic1_x, 1));

    let vk = VerificationKey {
        alpha_g1: g1_bytes(env, alpha_x, 1),
        beta_g2: g2_bytes(env, beta_x0),
        gamma_g2: g2_bytes(env, gamma_x0),
        delta_g2: g2_bytes(env, delta_x0),
        ic,
    };
    let proof = Proof {
        a: g1_bytes(env, a_x, 2),
        b: g2_bytes(env, b_x0),
        c: g1_bytes(env, c_x, 2),
    };
    let mut public_inputs = Vec::new(env);
    public_inputs.push_back(scalar_bytes(env, public_input));
    (vk, proof, public_inputs)
}

#[test]
fn test_register_and_verify_zk_proof() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);
    let (vk, proof, public_inputs) = valid_groth16(&env, 5);

    client.register_vk(&101u64, &vk);

    let result = client.verify_proof(&101u64, &proof, &public_inputs);
    assert_eq!(result, true);
    assert_eq!(client.get_proof_count(), 1);
}

#[test]
fn test_verify_fails_with_invalid_public_inputs() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);

    let mut ic = Vec::new(&env);
    ic.push_back(g1_bytes(&env, 1, 1));

    let vk = VerificationKey {
        alpha_g1: Bytes::new(&env),
        beta_g2: Bytes::new(&env),
        gamma_g2: Bytes::new(&env),
        delta_g2: Bytes::new(&env),
        ic,
    };

    client.register_vk(&202u64, &vk);

    let proof = Proof {
        a: Bytes::new(&env),
        b: Bytes::new(&env),
        c: Bytes::new(&env),
    };

    let public_inputs = Vec::new(&env); // IC has 1 element, public inputs has 0 elements -> mismatch
    let result = client.try_verify_proof(&202u64, &proof, &public_inputs);
    assert!(result.is_err());
}

#[test]
fn test_tampered_groth16_proof_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);
    let (vk, mut proof, public_inputs) = valid_groth16(&env, 5);
    client.register_vk(&303u64, &vk);

    // Tamper A.x so the pairing product no longer holds.
    proof.a = g1_bytes(&env, 2, 2);

    let result = client.verify_proof(&303u64, &proof, &public_inputs);
    assert_eq!(result, false);
    assert_eq!(client.get_proof_count(), 0);
}

#[test]
fn test_wrong_public_inputs_fail_pairing_check() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);
    let (vk, proof, _) = valid_groth16(&env, 5);
    client.register_vk(&404u64, &vk);

    let mut wrong_inputs = Vec::new(&env);
    wrong_inputs.push_back(scalar_bytes(&env, 99));

    let result = client.verify_proof(&404u64, &proof, &wrong_inputs);
    assert_eq!(result, false);
}

#[test]
fn test_circuit_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let (_id, _admin, client) = setup_verifier(&env);
    let (_vk, proof, public_inputs) = valid_groth16(&env, 1);
    let result = client.try_verify_proof(&999u64, &proof, &public_inputs);
    assert!(result.is_err());
}
