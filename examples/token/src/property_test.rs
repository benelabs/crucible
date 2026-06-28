//! Property‑style tests for token accounting invariants

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::{Token, TokenClient};
    use crucible::prelude::*;
    use soroban_sdk::{symbol_short, Address};
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        Mint { to: Address, amount: i128 },
        Transfer { from: Address, to: Address, amount: i128 },
        Burn { from: Address, amount: i128 },
        Approve { owner: Address, spender: Address, amount: i128 },
        TransferFrom { spender: Address, from: Address, to: Address, amount: i128 },
    }

    // Helper to generate random addresses within the mock env
    fn arb_address(env: &MockEnv) -> impl Strategy<Value = Address> {
        prop_oneof![
            Just(env.account("alice")),
            Just(env.account("bob")),
            Just(env.account("carol")),
            Just(env.account("dave")),
            Just(env.account("eve")),
        ]
    }

    // Generate a sequence of operations (deterministic length up to 10)
    fn operation_seq() -> impl Strategy<Value = Vec<Op>> {
        let len = 1..=10usize;
        len.prop_flat_map(|size| {
            prop::collection::vec(
                prop_oneof![
                    (arb_address(&MockEnv::default()), 1i128..1000i128).prop_map(|(to, amt)| Op::Mint { to, amount: amt }),
                    (arb_address(&MockEnv::default()), arb_address(&MockEnv::default()), 1i128..500i128).prop_map(|(from, to, amt)| Op::Transfer { from, to, amount: amt }),
                    (arb_address(&MockEnv::default()), 1i128..500i128).prop_map(|(from, amt)| Op::Burn { from, amount: amt }),
                    (arb_address(&MockEnv::default()), arb_address(&MockEnv::default()), 0i128..1000i128).prop_map(|(owner, spender, amt)| Op::Approve { owner, spender, amount: amt }),
                    (arb_address(&MockEnv::default()), arb_address(&MockEnv::default()), arb_address(&MockEnv::default()), 1i128..500i128).prop_map(|(spender, from, to, amt)| Op::TransferFrom { spender, from, to, amount: amt }),
                ],
                size,
            )
        })
    }

    #[test]
    fn deterministic_accounting_properties() {
        let env = MockEnv::builder()
            .with_contract::<Token>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("alice", Stroops::xlm(100))
            .with_account("bob", Stroops::xlm(100))
            .with_account("carol", Stroops::xlm(100))
            .build();
        let id = env.contract_id::<Token>();
        let admin = env.account("admin");
        env.mock_all_auths();
        TokenClient::new(env.inner(), &id).initialize(&admin);

        let ops = vec![
            Op::Mint { to: env.account("alice"), amount: 1000 },
            Op::Mint { to: env.account("bob"), amount: 500 },
            Op::Transfer { from: env.account("alice"), to: env.account("bob"), amount: 200 },
            Op::Approve { owner: env.account("bob"), spender: env.account("carol"), amount: 300 },
            Op::TransferFrom { spender: env.account("carol"), from: env.account("bob"), to: env.account("alice"), amount: 150 },
            Op::Burn { from: env.account("alice"), amount: 100 },
        ];

        let mut balances: std::collections::HashMap<Address, i128> = Default::default();
        let mut total_supply: i128 = 0;
        let mut allowances: std::collections::HashMap<(Address, Address), i128> = Default::default();

        for op in ops {
            match op {
                Op::Mint { to, amount } => {
                    TokenClient::new(env.inner(), &id).mint(&to, &amount);
                    *balances.entry(to.clone()).or_insert(0) += amount;
                    total_supply += amount;
                }
                Op::Transfer { from, to, amount } => {
                    TokenClient::new(env.inner(), &id).transfer(&from, &to, &amount);
                    *balances.entry(from.clone()).or_insert(0) -= amount;
                    *balances.entry(to.clone()).or_insert(0) += amount;
                }
                Op::Burn { from, amount } => {
                    TokenClient::new(env.inner(), &id).burn(&from, &amount);
                    *balances.entry(from.clone()).or_insert(0) -= amount;
                    total_supply -= amount;
                }
                Op::Approve { owner, spender, amount } => {
                    TokenClient::new(env.inner(), &id).approve(&owner, &spender, &amount);
                    allowances.insert((owner.clone(), spender.clone()), amount);
                }
                Op::TransferFrom { spender, from, to, amount } => {
                    TokenClient::new(env.inner(), &id).transfer_from(&spender, &from, &to, &amount);
                    let key = (from.clone(), spender.clone());
                    let remaining = allowances.get(&key).cloned().unwrap_or(0) - amount;
                    allowances.insert(key, remaining);
                    *balances.entry(from.clone()).or_insert(0) -= amount;
                    *balances.entry(to.clone()).or_insert(0) += amount;
                }
            }
            let sum_bal: i128 = balances.values().copied().sum();
            assert_eq!(total_supply, sum_bal, "total supply mismatch after op {:?}", op);
            for bal in balances.values() {
                assert!(*bal >= 0, "negative balance after op {:?}", op);
            }
        }
    }

    proptest! {
        #[test]
        fn fuzz_accounting_ops(ops in operation_seq()) {
            let env = MockEnv::builder()
                .with_contract::<Token>()
                .with_account("admin", Stroops::xlm(100))
                .with_account("alice", Stroops::xlm(100))
                .with_account("bob", Stroops::xlm(100))
                .with_account("carol", Stroops::xlm(100))
                .with_account("dave", Stroops::xlm(100))
                .with_account("eve", Stroops::xlm(100))
                .build();
            let id = env.contract_id::<Token>();
            let admin = env.account("admin");
            env.mock_all_auths();
            TokenClient::new(env.inner(), &id).initialize(&admin);

            let mut balances: std::collections::HashMap<Address, i128> = Default::default();
            let mut total_supply: i128 = 0;
            let mut allowances: std::collections::HashMap<(Address, Address), i128> = Default::default();

            for op in ops {
                let exec = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match &op {
                        Op::Mint { to, amount } => {
                            if *amount <= 0 { return; }
                            TokenClient::new(env.inner(), &id).mint(to, amount);
                            *balances.entry(to.clone()).or_insert(0) += amount;
                            total_supply += amount;
                        }
                        Op::Transfer { from, to, amount } => {
                            if *amount <= 0 { return; }
                            TokenClient::new(env.inner(), &id).transfer(from, to, amount);
                            *balances.entry(from.clone()).or_insert(0) -= amount;
                            *balances.entry(to.clone()).or_insert(0) += amount;
                        }
                        Op::Burn { from, amount } => {
                            if *amount <= 0 { return; }
                            TokenClient::new(env.inner(), &id).burn(from, amount);
                            *balances.entry(from.clone()).or_insert(0) -= amount;
                            total_supply -= amount;
                        }
                        Op::Approve { owner, spender, amount } => {
                            TokenClient::new(env.inner(), &id).approve(owner, spender, amount);
                            allowances.insert((owner.clone(), spender.clone()), *amount);
                        }
                        Op::TransferFrom { spender, from, to, amount } => {
                            if *amount <= 0 { return; }
                            TokenClient::new(env.inner(), &id).transfer_from(spender, from, to, amount);
                            let key = (from.clone(), spender.clone());
                            let remaining = allowances.get(&key).cloned().unwrap_or(0) - amount;
                            allowances.insert(key, remaining);
                            *balances.entry(from.clone()).or_insert(0) -= amount;
                            *balances.entry(to.clone()).or_insert(0) += amount;
                        }
                    }
                }));
                if exec.is_err() { continue; }
                let sum_bal: i128 = balances.values().copied().sum();
                assert_eq!(total_supply, sum_bal, "total supply mismatch after op {:?}", op);
                for bal in balances.values() {
                    assert!(*bal >= 0);
                }
                for al in allowances.values() {
                    assert!(*al >= 0);
                }
            }
        }
    }
}
