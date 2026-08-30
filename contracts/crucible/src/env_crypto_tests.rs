#[cfg(test)]
mod tests {
    use crate::env::{CryptoCurve, MockCryptoRegistry, MockEnv, MockKeyPair};
    use soroban_sdk::{contract, contractimpl, symbol_short, Bytes, BytesN, Env};

    #[contract]
    #[derive(Default)]
    struct SmartWalletContract;

    #[contractimpl]
    impl SmartWalletContract {
        pub fn verify_ed25519_auth(
            env: Env,
            public_key: BytesN<32>,
            message: Bytes,
            signature: BytesN<64>,
        ) -> bool {
            env.crypto().ed25519_verify(&public_key, &message, &signature);
            true
        }

        pub fn verify_secp256k1_auth(
            env: Env,
            public_key: BytesN<64>,
            message_digest: BytesN<32>,
            signature: BytesN<64>,
        ) -> bool {
            env.crypto().secp256k1_verify(&public_key, &message_digest, &signature);
            true
        }

        pub fn verify_secp256r1_auth(
            env: Env,
            public_key: BytesN<64>,
            message_digest: BytesN<32>,
            signature: BytesN<64>,
        ) -> bool {
            env.crypto().secp256r1_verify(&public_key, &message_digest, &signature);
            true
        }
    }

    #[test]
    fn test_ed25519_signature_verification_and_corrupt_injection() {
        let env = MockEnv::builder()
            .with_contract::<SmartWalletContract>()
            .build();
        let id = env.contract_id::<SmartWalletContract>();
        let client = SmartWalletContractClient::new(env.inner(), &id);

        let keypair = MockKeyPair::generate_ed25519(42);
        let pub_key = keypair.public_key_bytes32(env.inner());
        let message_bytes = Bytes::from_slice(env.inner(), b"authorize_tx_nonce_1001");

        // Valid signature verification
        let valid_sig = keypair.sign_bytes(env.inner(), &message_bytes);
        let verified = client.verify_ed25519_auth(&pub_key, &message_bytes, &valid_sig);
        assert!(verified, "Valid Ed25519 signature must pass verification");

        // Corrupt signature injection for negative test path
        let corrupt_sig = keypair.corrupt_signature_bytes(env.inner(), &message_bytes);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.verify_ed25519_auth(&pub_key, &message_bytes, &corrupt_sig);
        }));
        assert!(result.is_err(), "Corrupt Ed25519 signature must trigger verification panic");
    }

    #[test]
    fn test_secp256k1_signature_verification_and_corrupt_injection() {
        let env = MockEnv::builder()
            .with_contract::<SmartWalletContract>()
            .build();
        let id = env.contract_id::<SmartWalletContract>();
        let client = SmartWalletContractClient::new(env.inner(), &id);

        let keypair = MockKeyPair::generate_secp256k1(101);
        let pub_key = keypair.public_key_bytes64(env.inner());

        let digest_raw = env.inner().crypto().sha256(&Bytes::from_slice(env.inner(), b"tx_hash_secp256k1"));
        let message_digest = BytesN::from_array(env.inner(), &digest_raw.to_array());

        // Valid signature verification
        let valid_sig = keypair.sign(env.inner(), &digest_raw.to_array());
        let verified = client.verify_secp256k1_auth(&pub_key, &message_digest, &valid_sig);
        assert!(verified, "Valid Secp256k1 signature must pass verification");

        // Corrupt signature injection
        let corrupt_sig = keypair.corrupt_signature(env.inner(), &digest_raw.to_array());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.verify_secp256k1_auth(&pub_key, &message_digest, &corrupt_sig);
        }));
        assert!(result.is_err(), "Corrupt Secp256k1 signature must trigger verification panic");
    }

    #[test]
    fn test_secp256r1_signature_verification_and_corrupt_injection() {
        let env = MockEnv::builder()
            .with_contract::<SmartWalletContract>()
            .build();
        let id = env.contract_id::<SmartWalletContract>();
        let client = SmartWalletContractClient::new(env.inner(), &id);

        let keypair = MockKeyPair::generate_secp256r1(202);
        let pub_key = keypair.public_key_bytes64(env.inner());

        let digest_raw = env.inner().crypto().sha256(&Bytes::from_slice(env.inner(), b"tx_hash_secp256r1"));
        let message_digest = BytesN::from_array(env.inner(), &digest_raw.to_array());

        // Valid signature verification
        let valid_sig = keypair.sign(env.inner(), &digest_raw.to_array());
        let verified = client.verify_secp256r1_auth(&pub_key, &message_digest, &valid_sig);
        assert!(verified, "Valid Secp256r1 signature must pass verification");

        // Corrupt signature injection
        let corrupt_sig = keypair.corrupt_signature(env.inner(), &digest_raw.to_array());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.verify_secp256r1_auth(&pub_key, &message_digest, &corrupt_sig);
        }));
        assert!(result.is_err(), "Corrupt Secp256r1 signature must trigger verification panic");
    }

    #[test]
    fn test_mock_crypto_registry_lookup_and_env_integration() {
        let env = MockEnv::builder().build();

        // Register keypair via MockEnv helper
        let kp1 = env.generate_keypair(CryptoCurve::Ed25519, 77);
        assert_eq!(kp1.curve, CryptoCurve::Ed25519);

        // Registry management
        let mut registry = MockCryptoRegistry::new();
        registry.register("alice_wallet", kp1.clone());
        let fetched = registry.get("alice_wallet").expect("registered keypair should exist");
        assert_eq!(fetched.seed, 77);

        // Lazy keypair generation from registry
        let secp_kp = registry.secp256k1_keypair("bob_wallet", 88);
        assert_eq!(secp_kp.curve, CryptoCurve::Secp256k1);
        assert_eq!(secp_kp.seed, 88);
    }
}
