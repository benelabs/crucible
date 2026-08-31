#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use soroban_sdk::{symbol_short, Address};

use crate::{OptionPosition, OptionStatus, OptionType, OptionsProtocol, OptionsProtocolClient};

const BASE_TIME: u64 = 1_000_000;
const OPTION_EXPIRY: u64 = BASE_TIME + 86_400;

struct Ctx {
    env: MockEnv,
    id: Address,
    writer: AccountHandle,
    holder: AccountHandle,
    token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<OptionsProtocol>()
            .with_account("writer", Stroops::xlm(1000))
            .with_account("holder", Stroops::xlm(1000))
            .build();

        let token = MockToken::new(&env, "USDC", 6);
        let id = env.contract_id::<OptionsProtocol>();
        let writer = env.account("writer");
        let holder = env.account("holder");
        token.mint(&writer, 1_000_000_000);

        Ctx { env, id, writer, holder, token }
    }

    fn client(&self) -> OptionsProtocolClient<'_> {
        OptionsProtocolClient::new(self.env.inner(), &self.id)
    }
}

#[test]
fn test_call_option_exercise_in_the_money() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| {
        ctx.client().initialize(&ctx.writer, &ctx.token.address());
        ctx.client().mint(
            &ctx.writer,
            &ctx.holder,
            &OptionType::Call,
            &500_i128,
            &50_i128,
            &10_i128,
            &1_000_i128,
            &(OPTION_EXPIRY),
        );
    });

    ctx.env.advance_time(Duration::seconds(10));
    ctx.env.with_mock_all_auths(|| ctx.client().exercise(&ctx.holder));

    assert_eq!(ctx.client().get_state().status, OptionStatus::Exercised);
    assert_eq!(ctx.token.balance(&ctx.holder), 1_000_000_000);
}

#[test]
fn test_put_option_expires_out_of_the_money() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| {
        ctx.client().initialize(&ctx.writer, &ctx.token.address());
        ctx.client().mint(
            &ctx.writer,
            &ctx.holder,
            &OptionType::Put,
            &500_i128,
            &50_i128,
            &10_i128,
            &10_000_i128,
            &(OPTION_EXPIRY),
        );
    });

    ctx.env.advance_time(Duration::seconds(10));
    ctx.env.with_mock_all_auths(|| ctx.client().exercise(&ctx.holder));

    assert_eq!(ctx.client().get_state().status, OptionStatus::Expired);
}
