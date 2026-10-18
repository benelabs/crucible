#[cfg(test)]
mod tests {
    use crate::env::{CryptoCurve, MockCryptoRegistry, MockEnv, MockKeyPair};
    use soroban_sdk::{contract, contractimpl, Bytes, BytesN, Env};

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

        /// Soroban exposes no `secp256k1_verify` host function; verification is
        /// expressed as "recover the key from the signature and compare".
        pub fn verify_secp256k1_auth(
            env: Env,
            public_key: BytesN<65>,
            message: Bytes,
            signature: BytesN<64>,
            recovery_id: u32,
        ) -> bool {
            // Hashing in-contract is what yields a trusted `Hash<32>`; the host
            // refuses to accept one directly as an entrypoint argument.
            let digest = env.crypto().sha256(&message);
            let recovered = env
                .crypto()
                .secp256k1_recover(&digest, &signature, recovery_id);
            recovered == public_key
        }

        pub fn verify_secp256r1_auth(
            env: Env,
            public_key: BytesN<65>,
            message: Bytes,
            signature: BytesN<64>,
        ) -> bool {
            let digest = env.crypto().sha256(&message);
            env.crypto()
                .secp256r1_verify(&public_key, &digest, &signature);
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
        let pub_key = keypair.public_key_sec1_bytes65(env.inner());

        let message = Bytes::from_slice(env.inner(), b"tx_hash_secp256k1");
        let digest_array = env.inner().crypto().sha256(&message).to_array();

        // Valid signature recovers exactly the signing key.
        let (valid_sig, recovery_id) =
            keypair.sign_prehash_recoverable(env.inner(), &digest_array);
        let verified =
            client.verify_secp256k1_auth(&pub_key, &message, &valid_sig, &recovery_id);
        assert!(verified, "Valid Secp256k1 signature must pass verification");

        // A corrupt signature must not recover the signing key. The host either
        // rejects the malformed signature outright (panic) or recovers some
        // other key; both outcomes count as a failed verification.
        let mut corrupt_bytes = valid_sig.to_array();
        corrupt_bytes[63] ^= 0xff;
        let corrupt_sig = BytesN::from_array(env.inner(), &corrupt_bytes);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.verify_secp256k1_auth(&pub_key, &message, &corrupt_sig, &recovery_id)
        }));
        assert!(
            !matches!(result, Ok(true)),
            "Corrupt Secp256k1 signature must fail verification"
        );
    }

    #[test]
    fn test_secp256r1_signature_verification_and_corrupt_injection() {
        let env = MockEnv::builder()
            .with_contract::<SmartWalletContract>()
            .build();
        let id = env.contract_id::<SmartWalletContract>();
        let client = SmartWalletContractClient::new(env.inner(), &id);

        let keypair = MockKeyPair::generate_secp256r1(202);
        let pub_key = keypair.public_key_sec1_bytes65(env.inner());

        let message = Bytes::from_slice(env.inner(), b"tx_hash_secp256r1");
        let digest_array = env.inner().crypto().sha256(&message).to_array();

        // Valid signature verification
        let valid_sig = keypair.sign_prehash(env.inner(), &digest_array);
        let verified = client.verify_secp256r1_auth(&pub_key, &message, &valid_sig);
        assert!(verified, "Valid Secp256r1 signature must pass verification");

        // Corrupt signature injection
        let mut corrupt_bytes = valid_sig.to_array();
        corrupt_bytes[63] ^= 0xff;
        let corrupt_sig = BytesN::from_array(env.inner(), &corrupt_bytes);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.verify_secp256r1_auth(&pub_key, &message, &corrupt_sig);
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
