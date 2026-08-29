use arbitrary::Arbitrary;
use crucible::env::{MockEnv, Stroops};
use crucible::token::MockToken;

#[derive(Arbitrary, Debug)]
struct EnvConfig {
    sequence: u32,
    timestamp: u64,
    accounts: Vec<(String, u64)>,
}

#[test]
fn test_property_env_config_roundtrip() {
    let raw_data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let mut unstructured = arbitrary::Unstructured::new(&raw_data);
    if let Ok(config) = EnvConfig::arbitrary(&mut unstructured) {
        let mut builder = MockEnv::builder()
            .at_sequence(config.sequence)
            .at_timestamp(config.timestamp);

        let mut seen_names = std::collections::HashSet::new();
        for (name, amount) in &config.accounts {
            if !name.trim().is_empty() && seen_names.insert(name.clone()) {
                let stroops = Stroops::from((*amount as i128).abs());
                builder = builder.with_account(name, stroops);
            }
        }
        let env = builder.build();
        assert_eq!(env.timestamp(), config.timestamp);
    }
}

#[test]
fn test_property_sequence_timestamp_max_bounds() {
    let env = MockEnv::builder()
        .at_sequence(u32::MAX)
        .at_timestamp(u64::MAX)
        .build();
    assert_eq!(env.timestamp(), u64::MAX);
}

#[test]
fn test_property_account_balance_large_stroops() {
    let max_stroops = Stroops::from(i128::MAX);
    let env = MockEnv::builder()
        .with_account("whales", max_stroops)
        .build();
    let acc = env.account("whales");
    assert!(!acc.address().to_string().is_empty());
}

#[test]
fn test_property_duplicate_account_names() {
    let result = std::panic::catch_unwind(|| {
        let _ = MockEnv::builder()
            .with_account("alice", Stroops::from(100))
            .with_account("alice", Stroops::from(200))
            .build();
    });
    assert!(result.is_err(), "Duplicate account names must panic predictably");
}

#[test]
fn test_property_empty_symbol_string() {
    let env = MockEnv::builder().build();
    let result = std::panic::catch_unwind(|| {
        let _ = MockToken::new(&env, "", 6);
    });
    assert!(result.is_err(), "Empty symbol string must fail with a clear message");
}

#[test]
fn test_property_stroops_from_xlm_str_fuzz() {
    let inputs = vec!["100.5", "0.0000001", "999999999", "invalid", "", "-10"];
    for input in inputs {
        let _ = std::panic::catch_unwind(|| {
            Stroops::from_xlm_str(input);
        });
    }
}
