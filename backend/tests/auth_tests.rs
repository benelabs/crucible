use backend::api::handlers::auth::jwt::JwtKeyManager;

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
