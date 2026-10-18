#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crate::{PaymentStreaming, PaymentStreamingClient};

#[test]
fn test_payment_streaming_lifecycle() {
    let env = MockEnv::builder()
        .at_timestamp(1_000_000)
        .with_contract::<PaymentStreaming>()
        .with_account("sender", Stroops::xlm(100))
        .with_account("recipient", Stroops::xlm(10))
        .build();

    let contract_id = env.contract_id::<PaymentStreaming>();
    let sender = env.account("sender");
    let recipient = env.account("recipient");

    let token = MockToken::new(&env, "USDC", 6);
    token.mint(&sender, 10_000_000);

    let client = PaymentStreamingClient::new(env.inner(), &contract_id);

    // Create stream: 1,000,000 to 2,000,000 (duration: 1,000,000 seconds)
    let stream_id = client.create_stream(
        &sender.address(),
        &recipient.address(),
        &token.address(),
        &10_000_000,
        &1_000_000,
        &2_000_000,
    );

    assert_eq!(stream_id, 1);

    // Halfway through stream (advance by 500,000 seconds)
    env.advance_time(Duration::seconds(500_000));

    let claimable = client.claimable_amount(&stream_id);
    assert_eq!(claimable, 5_000_000);

    // Withdraw 50%
    let withdrawn = client.withdraw_from_stream(&recipient.address(), &stream_id);
    assert_eq!(withdrawn, 5_000_000);
    assert_eq!(token.balance(&recipient), 5_000_000);

    // Advance to end
    env.advance_time(Duration::seconds(500_000));
    let final_withdrawn = client.withdraw_from_stream(&recipient.address(), &stream_id);
    assert_eq!(final_withdrawn, 5_000_000);
    assert_eq!(token.balance(&recipient), 10_000_000);
}
