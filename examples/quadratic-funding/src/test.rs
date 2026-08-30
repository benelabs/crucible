#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use soroban_sdk::{symbol_short, Address, String};

use crate::{ProjectProposal, QuadraticFundingDistributor, QuadraticFundingDistributorClient};

const MATCHING_POOL: i128 = 100_000;
const ROUND_START: u64 = 1_000;
const ROUND_END: u64 = 5_000;
const CLAIM_PERIOD_END: u64 = 10_000;

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub admin: AccountHandle,
    pub recipient_a: AccountHandle,
    pub recipient_b: AccountHandle,
    pub donor_1: AccountHandle,
    pub donor_2: AccountHandle,
    pub donor_3: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(ROUND_START)
            .with_contract::<QuadraticFundingDistributor>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("rec_a", Stroops::xlm(10))
            .with_account("rec_b", Stroops::xlm(10))
            .with_account("d1", Stroops::xlm(10))
            .with_account("d2", Stroops::xlm(10))
            .with_account("d3", Stroops::xlm(10))
            .build();

        let id = env.contract_id::<QuadraticFundingDistributor>();
        let admin = env.account("admin");
        let recipient_a = env.account("rec_a");
        let recipient_b = env.account("rec_b");
        let donor_1 = env.account("d1");
        let donor_2 = env.account("d2");
        let donor_3 = env.account("d3");
        let token = MockToken::new(&env, "USDC", 6);

        token.mint(&admin, MATCHING_POOL * 2);
        token.mint(&donor_1, 10_000);
        token.mint(&donor_2, 10_000);
        token.mint(&donor_3, 10_000);

        Ctx {
            env,
            id,
            admin,
            recipient_a,
            recipient_b,
            donor_1,
            donor_2,
            donor_3,
            token,
        }
    }

    fn client(&self) -> QuadraticFundingDistributorClient<'_> {
        QuadraticFundingDistributorClient::new(self.env.inner(), &self.id)
    }

    fn init_round(&self) {
        self.env.with_mock_all_auths(|| {
            self.client().initialize(
                &self.admin,
                &self.token.address(),
                &MATCHING_POOL,
                &ROUND_START,
                &ROUND_END,
                &CLAIM_PERIOD_END,
            );
        });
    }
}

#[test]
fn test_quadratic_funding_broad_support_vs_single_whale() {
    let ctx = Ctx::setup();
    ctx.init_round();

    // Register 2 projects
    let proj_a = ctx.env.with_mock_all_auths(|| {
        ctx.client().register_project(
            &ctx.recipient_a,
            &String::from_str(ctx.env.inner(), "Broad Support Project"),
        )
    });

    let proj_b = ctx.env.with_mock_all_auths(|| {
        ctx.client().register_project(
            &ctx.recipient_b,
            &String::from_str(ctx.env.inner(), "Single Whale Project"),
        )
    });

    // Proj A gets 100 each from 2 donors (sum sqrt = sqrt(100) + sqrt(100) = 10 + 10 = 20, (20)^2 = 400)
    ctx.env.with_mock_all_auths(|| {
        ctx.client().contribute(&ctx.donor_1, &proj_a, &100);
        ctx.client().contribute(&ctx.donor_2, &proj_a, &100);
    });

    // Proj B gets 200 from single donor 3 (sum sqrt = sqrt(200) = 14, (14)^2 = 196)
    ctx.env.with_mock_all_auths(|| {
        ctx.client().contribute(&ctx.donor_3, &proj_b, &200);
    });

    // Advance time past round end
    ctx.env.set_timestamp(ROUND_END + 100);

    // Claim payout for Proj A and Proj B
    let (direct_a, match_a) = ctx
        .env
        .with_mock_all_auths(|| ctx.client().claim_payout(&proj_a));
    let (direct_b, match_b) = ctx
        .env
        .with_mock_all_auths(|| ctx.client().claim_payout(&proj_b));

    assert_eq!(direct_a, 200);
    assert_eq!(direct_b, 200);

    // Project A has broader community consensus, so matching grant subsidy is significantly higher
    assert!(match_a > match_b);
    assert_eq!(
        ctx.token.balance(&ctx.recipient_a),
        direct_a + match_a
    );
    assert_eq!(
        ctx.token.balance(&ctx.recipient_b),
        direct_b + match_b
    );
}

#[test]
fn test_timeline_enforcement() {
    let ctx = Ctx::setup();
    ctx.init_round();

    let proj_id = ctx.env.with_mock_all_auths(|| {
        ctx.client().register_project(
            &ctx.recipient_a,
            &String::from_str(ctx.env.inner(), "Test Project"),
        )
    });

    // Claim before round end must revert
    let early_claim = ctx.env.with_mock_all_auths(|| {
        ctx.client().try_claim_payout(&proj_id)
    });
    assert!(early_claim.is_err(), "Cannot claim before round ends");

    // Advance beyond claim period end
    ctx.env.set_timestamp(CLAIM_PERIOD_END + 1);
    let late_claim = ctx.env.with_mock_all_auths(|| {
        ctx.client().try_claim_payout(&proj_id)
    });
    assert!(late_claim.is_err(), "Cannot claim after claim period expired");
}
