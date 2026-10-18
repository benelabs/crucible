use backend::api::handlers::auth::jwt::JwtKeyManager;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};

#[test]
fn generated_jwt_keys_are_valid_and_jwks_contains_rsa_components() {
    let key_manager = JwtKeyManager::new();
    let key_pair = key_manager.active_key().expect("Initial key should exist");
    let private_key = RsaPrivateKey::from_pkcs8_pem(&key_pair.private_key_pem)
        .expect("private key should be valid PKCS#8 PEM");
    let public_key = RsaPublicKey::from_public_key_pem(&key_pair.public_key_pem)
        .expect("public key should be valid PEM");

    assert_eq!(public_key, RsaPublicKey::from(&private_key));

    let jwks = key_manager.get_jwks();
    let jwk = &jwks.keys[0];
    assert_eq!(jwk.kty, "RSA");
    assert_eq!(jwk.alg, "RS256");
    assert_eq!(URL_SAFE_NO_PAD.decode(&jwk.n).unwrap(), public_key.n().to_bytes_be());
    assert_eq!(URL_SAFE_NO_PAD.decode(&jwk.e).unwrap(), public_key.e().to_bytes_be());

    let jwks_json = serde_json::to_value(&jwks).unwrap();
    assert_eq!(jwks_json["keys"][0]["use"], "sig");
    assert!(jwks_json["keys"][0].get("use_").is_none());

    let serialized_key_pair = serde_json::to_string(&key_pair).unwrap();
    assert!(!serialized_key_pair.contains("PRIVATE KEY"));

    let debug_key_pair = format!("{key_pair:?}");
    assert!(!debug_key_pair.contains(&key_pair.private_key_pem));
    assert!(debug_key_pair.contains("[REDACTED]"));
}

#[tokio::test]
async fn test_jwt_key_rotation() {
    let key_manager = JwtKeyManager::new();

    let initial_key = key_manager.active_key().expect("Initial key should exist");
    assert_eq!(initial_key.kid, "key-v1");

    let jwks_initial = key_manager.get_jwks();
    assert_eq!(jwks_initial.keys.len(), 1);

    let rotated_key = key_manager.rotate_keys();
    assert_ne!(rotated_key.kid, initial_key.kid);

    let active = key_manager.active_key().expect("Active key after rotation");
    assert_eq!(active.kid, rotated_key.kid);

    let jwks_rotated = key_manager.get_jwks();
    assert_eq!(jwks_rotated.keys.len(), 2);
}
