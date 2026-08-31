#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use soroban_sdk::{symbol_short, vec, Address, Vec};

use crate::{CallInstruction, SmartWallet, SmartWalletClient};

const DAILY_LIMIT: i128 = 1_000_000;

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub owner: AccountHandle,
    pub guardian_1: AccountHandle,
    pub guardian_2: AccountHandle,
    pub guardian_3: AccountHandle,
    pub new_owner: AccountHandle,
    pub recipient: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(1_000_000)
            .with_contract::<SmartWallet>()
            .with_account("owner", Stroops::xlm(100))
            .with_account("g1", Stroops::xlm(10))
            .with_account("g2", Stroops::xlm(10))
            .with_account("g3", Stroops::xlm(10))
            .with_account("new_owner", Stroops::xlm(10))
            .with_account("recipient", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<SmartWallet>();
        let owner = env.account("owner");
        let guardian_1 = env.account("g1");
        let guardian_2 = env.account("g2");
        let guardian_3 = env.account("g3");
        let new_owner = env.account("new_owner");
        let recipient = env.account("recipient");
        let token = MockToken::new(&env, "USDC", 6);

        // Fund smart wallet contract
        token.mint_to_address(&id, DAILY_LIMIT * 5);

        Ctx {
            env,
            id,
            owner,
            guardian_1,
            guardian_2,
            guardian_3,
            new_owner,
            recipient,
            token,
        }
    }

    fn client(&self) -> SmartWalletClient<'_> {
        SmartWalletClient::new(self.env.inner(), &self.id)
    }

    fn init_wallet(&self) {
        let guardians = vec![
            self.env.inner(),
            self.guardian_1.address(),
            self.guardian_2.address(),
            self.guardian_3.address(),
        ];

        self.env.with_mock_all_auths(|| {
            self.client().initialize(
                &self.owner,
                &guardians,
                &2u32, // 2-of-3 threshold
                &DAILY_LIMIT,
            );
        });
    }
}

#[test]
fn test_daily_spending_limit_enforcement() {
    let ctx = Ctx::setup();
    ctx.init_wallet();

    // Spend within limit
    ctx.env.with_mock_all_auths(|| {
        ctx.client().execute_transfer(
            &ctx.owner,
            &ctx.token.address(),
            &ctx.recipient,
            &600_000,
        );
    });

    assert_eq!(ctx.token.balance(&ctx.recipient), 600_000);

    // Spend exceeding remaining limit in same day should revert
    let res = ctx.env.with_mock_all_auths(|| {
        ctx.client().try_execute_transfer(
            &ctx.owner,
            &ctx.token.address(),
            &ctx.recipient,
            &500_000,
        )
    });
    assert!(res.is_err(), "Daily limit exceeded must revert");
}

#[test]
fn test_batched_calls_execution() {
    let ctx = Ctx::setup();
    ctx.init_wallet();

    let calls = vec![
        ctx.env.inner(),
        CallInstruction {
            token: ctx.token.address(),
            recipient: ctx.recipient.address(),
            amount: 100_000,
        },
        CallInstruction {
            token: ctx.token.address(),
            recipient: ctx.recipient.address(),
            amount: 200_000,
        },
    ];

    ctx.env.with_mock_all_auths(|| {
        ctx.client().execute_batch(&ctx.owner, &calls);
    });

    assert_eq!(ctx.token.balance(&ctx.recipient), 300_000);
}

#[test]
fn test_social_recovery_by_guardian_quorum() {
    let ctx = Ctx::setup();
    ctx.init_wallet();

    assert_eq!(ctx.client().get_owner(), ctx.owner.address());

    // Guardian 1 votes for new owner (1 of 2 votes)
    ctx.env.with_mock_all_auths(|| {
        ctx.client().approve_recovery(&ctx.guardian_1, &ctx.new_owner);
    });
    assert_eq!(ctx.client().get_owner(), ctx.owner.address());

    // Guardian 2 votes for new owner (2 of 2 quorum reached)
    ctx.env.with_mock_all_auths(|| {
        ctx.client().approve_recovery(&ctx.guardian_2, &ctx.new_owner);
    });

    // Ownership recovered to new_owner
    assert_eq!(ctx.client().get_owner(), ctx.new_owner.address());
}
