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

/// G1 point on the mock curve y² = x³ + 3. Known valid pair: (1, 2).
fn g1_on_curve(env: &Env, x: u64, y: u64) -> Bytes {
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
    // Non-zero y limb so the point is not infinity / torsion-trivial.
    raw[16..24].copy_from_slice(&1u64.to_le_bytes());
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
/// with A.x = 1 so B.x0 = rhs. All G1 points use the on-curve pair (1, 2).
fn valid_groth16(env: &Env, public_input: u64) -> (VerificationKey, Proof, Vec<Bytes>) {
    let alpha_x = 1u64;
    let beta_x0 = 5u64;
    let gamma_x0 = 17u64;
    let delta_x0 = 19u64;
    let ic0_x = 1u64;
    let ic1_x = 1u64;
    let a_x = 1u64;
    let c_x = 1u64;
    let l_x = ic0_x.wrapping_add(public_input.wrapping_mul(ic1_x));
    let b_x0 = alpha_x
        .wrapping_mul(beta_x0)
        .wrapping_add(l_x.wrapping_mul(gamma_x0))
        .wrapping_add(c_x.wrapping_mul(delta_x0));

    let mut ic = Vec::new(env);
    // IC points must be on-curve; x=1,y=2 works. Pairing only reads the x limb.
    ic.push_back(g1_on_curve(env, ic0_x, 2));
    ic.push_back(g1_on_curve(env, ic1_x, 2));

    let vk = VerificationKey {
        alpha_g1: g1_on_curve(env, alpha_x, 2),
        beta_g2: g2_bytes(env, beta_x0),
        gamma_g2: g2_bytes(env, gamma_x0),
        delta_g2: g2_bytes(env, delta_x0),
        ic,
    };
    let proof = Proof {
        a: g1_on_curve(env, a_x, 2),
        b: g2_bytes(env, b_x0),
        c: g1_on_curve(env, c_x, 2),
    };
    let mut public_inputs = Vec::new(env);
    let safe = if public_input == 0 { 5 } else { public_input };
    public_inputs.push_back(scalar_bytes(env, safe));
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
    ic.push_back(g1_on_curve(&env, 1, 2));
    ic.push_back(g1_on_curve(&env, 1, 2));

    let vk = VerificationKey {
        alpha_g1: g1_on_curve(&env, 1, 2),
        beta_g2: g2_bytes(&env, 5),
        gamma_g2: g2_bytes(&env, 7),
        delta_g2: g2_bytes(&env, 11),
        ic,
    };

    client.register_vk(&202u64, &vk);

    let proof = Proof {
        a: g1_on_curve(&env, 1, 2),
        b: g2_bytes(&env, 5),
        c: g1_on_curve(&env, 1, 2),
    };

    let public_inputs = Vec::new(&env); // IC has 2, inputs 0 → mismatch
    let result = client.try_verify_proof(&202u64, &proof, &public_inputs);
    assert!(result.is_err());
}

#[test]
fn test_infinity_point_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);
    let (mut vk, _proof, _public_inputs) = valid_groth16(&env, 5);
    let mut inf = Bytes::new(&env);
    inf.extend_from_array(&[0u8; 64]);
    vk.ic.set(0, inf);

    assert!(client.try_register_vk(&505u64, &vk).is_err());
}

#[test]
fn test_off_curve_point_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);
    let (vk, mut proof, public_inputs) = valid_groth16(&env, 5);
    client.register_vk(&606u64, &vk);

    // (3, 3): 9 != 27+3 — off curve
    proof.a = g1_on_curve(&env, 3, 3);
    assert!(client.try_verify_proof(&606u64, &proof, &public_inputs).is_err());
}

#[test]
fn test_tampered_groth16_proof_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);
    let (vk, mut proof, public_inputs) = valid_groth16(&env, 5);
    client.register_vk(&303u64, &vk);

    // Keep A on-curve at (1,2) but mutate B's x limb so the pairing product fails.
    let mut raw = [0u8; 128];
    raw[0..8].copy_from_slice(&2u64.to_le_bytes());
    raw[16..24].copy_from_slice(&1u64.to_le_bytes());
    let mut b = Bytes::new(&env);
    b.extend_from_array(&raw);
    proof.b = b;

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
