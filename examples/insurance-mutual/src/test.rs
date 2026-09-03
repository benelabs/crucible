#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crate::{InsuranceMutual, InsuranceMutualClient};

#[test]
fn test_insurance_mutual_workflow() {
    let env = MockEnv::builder()
        .at_timestamp(1_000_000)
        .with_contract::<InsuranceMutual>()
        .with_account("underwriter", Stroops::xlm(100))
        .with_account("policyholder", Stroops::xlm(100))
        .with_account("assessor", Stroops::xlm(10))
        .build();

    let contract_id = env.contract_id::<InsuranceMutual>();
    let underwriter = env.account("underwriter");
    let policyholder = env.account("policyholder");
    let assessor = env.account("assessor");

    let token = MockToken::new(&env, "USDC", 6);
    token.mint(&underwriter, 50_000_000);
    token.mint(&policyholder, 5_000_000);

    let client = InsuranceMutualClient::new(env.inner(), &contract_id);

    // Initialize with 20% reserve ratio
    client.initialize(&token.address(), &2000);

    // Deposit 50 USDC capital into risk pool
    client.deposit_capital(&underwriter.address(), &50_000_000);

    // Buy policy: 100 USDC coverage, 5 USDC premium, 30 days duration
    let policy_id = client.buy_policy(
        &policyholder.address(),
        &100_000_000,
        &5_000_000,
        &2_592_000,
    );
    assert_eq!(policy_id, 1);

    // Submit claim for 20 USDC
    let claim_id = client.submit_claim(&policyholder.address(), &policy_id, &20_000_000);
    assert_eq!(claim_id, 1);

    // Assessor votes to approve claim
    client.vote_and_process_claim(&assessor.address(), &claim_id, &true, &150);

    // Verify claim payout received
    assert_eq!(token.balance(&policyholder), 20_000_000);
}
