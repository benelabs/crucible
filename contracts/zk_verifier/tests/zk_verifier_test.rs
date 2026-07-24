#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, Vec};
use zk_verifier::{Proof, VerificationKey, ZkError, ZkVerifier, ZkVerifierClient};

fn setup_verifier(env: &Env) -> (Address, Address, ZkVerifierClient) {
    let admin = Address::generate(env);
    let contract_id = env.register(ZkVerifier, ());
    let client = ZkVerifierClient::new(env, &contract_id);
    client.initialize(&admin);
    (contract_id, admin, client)
}

#[test]
fn test_register_and_verify_zk_proof() {
    let env = Env::default();
    env.mock_all_auths();

    let (_id, _admin, client) = setup_verifier(&env);

    let mut ic = Vec::new(&env);
    let mut ic_bytes1 = Bytes::new(&env);
    ic_bytes1.extend_from_array(&[1u8; 64]);
    let mut ic_bytes2 = Bytes::new(&env);
    ic_bytes2.extend_from_array(&[2u8; 64]);
    ic.push_back(ic_bytes1);
    ic.push_back(ic_bytes2);

    let mut alpha = Bytes::new(&env);
    alpha.extend_from_array(&[1u8; 64]);
    let mut beta = Bytes::new(&env);
    beta.extend_from_array(&[2u8; 128]);
    let mut gamma = Bytes::new(&env);
    gamma.extend_from_array(&[3u8; 128]);
    let mut delta = Bytes::new(&env);
    delta.extend_from_array(&[4u8; 128]);

    let vk = VerificationKey {
        alpha_g1: alpha,
        beta_g2: beta,
        gamma_g2: gamma,
        delta_g2: delta,
        ic,
    };

    client.register_vk(&101u64, &vk);

    // Create proof
    let mut a = Bytes::new(&env);
    a.extend_from_array(&[10u8; 64]);
    let mut b = Bytes::new(&env);
    b.extend_from_array(&[20u8; 128]);
    let mut c = Bytes::new(&env);
    c.extend_from_array(&[30u8; 64]);

    let proof = Proof { a, b, c };

    let mut public_inputs = Vec::new(&env);
    let mut pub_in = Bytes::new(&env);
    pub_in.extend_from_array(&[99u8; 32]);
    public_inputs.push_back(pub_in);

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
    let mut ic_bytes = Bytes::new(&env);
    ic_bytes.extend_from_array(&[1u8; 64]);
    ic.push_back(ic_bytes);

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
