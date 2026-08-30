#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, Address, Option, String};

use crate::{NFTRentalProtocol, NFTRentalProtocolClient, RentalAgreement, RentalListing, NFT};

const RENTAL_FEE: i128 = 50_000;
const COLLATERAL: i128 = 200_000;
const MAX_DURATION: u64 = 7 * 86_400; // 7 days
const RENTAL_DURATION: u64 = 3 * 86_400; // 3 days
const BASE_TIME: u64 = 1_000_000;

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub owner: AccountHandle,
    pub renter: AccountHandle,
    pub stranger: AccountHandle,
    pub token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(BASE_TIME)
            .with_contract::<NFTRentalProtocol>()
            .with_account("owner", Stroops::xlm(100))
            .with_account("renter", Stroops::xlm(100))
            .with_account("stranger", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<NFTRentalProtocol>();
        let owner = env.account("owner");
        let renter = env.account("renter");
        let stranger = env.account("stranger");
        let token = MockToken::new(&env, "USDC", 6);
        token.mint(&renter, (RENTAL_FEE + COLLATERAL) * 3);

        Ctx {
            env,
            id,
            owner,
            renter,
            stranger,
            token,
        }
    }

    fn client(&self) -> NFTRentalProtocolClient<'_> {
        NFTRentalProtocolClient::new(self.env.inner(), &self.id)
    }

    fn mint_and_list(&self) -> u64 {
        let uri = String::from_str(self.env.inner(), "ipfs://QmSwordOfValor");
        let token_id = self
            .env
            .with_mock_all_auths(|| self.client().mint(&self.owner, &uri));

        self.env.with_mock_all_auths(|| {
            self.client().list_for_rent(
                &token_id,
                &self.owner,
                &self.token.address(),
                &RENTAL_FEE,
                &COLLATERAL,
                &MAX_DURATION,
            );
        });

        token_id
    }
}

// ---------------------------------------------------------------------------
// Minting & Listing
// ---------------------------------------------------------------------------

#[test]
fn test_mint_and_list_nft() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    let nft: NFT = ctx.client().get_nft(&token_id);
    assert_eq!(nft.token_id, token_id);
    assert_eq!(nft.owner, ctx.owner.address());

    let listing: RentalListing = ctx.client().get_listing(&token_id);
    assert_eq!(listing.token_id, token_id);
    assert_eq!(listing.owner, ctx.owner.address());
    assert_eq!(listing.rental_fee, RENTAL_FEE);
    assert_eq!(listing.collateral_amount, COLLATERAL);
    assert_eq!(listing.is_listed, true);
}

// ---------------------------------------------------------------------------
// Owner Rights vs User Rights Separation
// ---------------------------------------------------------------------------

#[test]
fn test_separation_of_owner_and_user_rights_during_rental() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    // Before rental: owner is owner, user is None
    assert_eq!(ctx.client().get_owner(&token_id), ctx.owner.address());
    assert_eq!(ctx.client().get_user(&token_id), Option::None);
    assert_eq!(ctx.client().is_rented(&token_id), false);

    // Rent NFT
    ctx.env.with_mock_all_auths(|| {
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION);
    });

    // During rental:
    // 1. Owner rights remain strictly with owner (never transferred)
    assert_eq!(ctx.client().get_owner(&token_id), ctx.owner.address());

    // 2. Usability rights are held by renter
    assert_eq!(
        ctx.client().get_user(&token_id),
        Option::Some(ctx.renter.address())
    );
    assert_eq!(ctx.client().is_rented(&token_id), true);

    // 3. Token balances: owner received fee, contract holds collateral escrow
    assert_eq!(ctx.token.balance(&ctx.owner), RENTAL_FEE);
    assert_eq!(ctx.token.balance(&ctx.id), COLLATERAL);
}

// ---------------------------------------------------------------------------
// Automatic Usability Expiry
// ---------------------------------------------------------------------------

#[test]
fn test_automatic_reclamation_of_usability_rights_on_expiry() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION);
    });

    // Advance time past rental expiry
    ctx.env
        .advance_time(Duration::seconds(RENTAL_DURATION + 1));

    // Usability rights automatically expire without requiring contract state mutation
    assert_eq!(ctx.client().get_user(&token_id), Option::None);
    assert_eq!(ctx.client().is_rented(&token_id), false);

    // Owner rights remain intact
    assert_eq!(ctx.client().get_owner(&token_id), ctx.owner.address());

    // Reclaim expired rental and refund collateral
    ctx.env.with_mock_all_auths(|| {
        ctx.client().reclaim_expired(&token_id);
    });

    assert_eq!(ctx.token.balance(&ctx.renter), (RENTAL_FEE + COLLATERAL) * 3 - RENTAL_FEE);
    assert_eq!(ctx.token.balance(&ctx.id), 0);
}

// ---------------------------------------------------------------------------
// Early Rental Termination & Collateral Refund
// ---------------------------------------------------------------------------

#[test]
fn test_early_rental_termination_and_collateral_refund() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION);
    });

    // 1 day into the 3-day rental, renter terminates early
    ctx.env.advance_time(Duration::seconds(86_400));
    assert_eq!(
        ctx.client().get_user(&token_id),
        Option::Some(ctx.renter.address())
    );

    let initial_renter_bal = ctx.token.balance(&ctx.renter);

    ctx.env.with_mock_all_auths(|| {
        ctx.client().terminate_early(&token_id, &ctx.renter);
    });

    // Usability rights immediately revoked
    assert_eq!(ctx.client().get_user(&token_id), Option::None);
    assert_eq!(ctx.client().is_rented(&token_id), false);

    // Collateral refunded immediately to renter
    assert_eq!(
        ctx.token.balance(&ctx.renter),
        initial_renter_bal + COLLATERAL
    );
    assert_eq!(ctx.token.balance(&ctx.id), 0);

    let agreement: RentalAgreement = ctx.client().get_rental(&token_id);
    assert_eq!(agreement.is_active, false);
}

// ---------------------------------------------------------------------------
// Edge Cases & Reverts
// ---------------------------------------------------------------------------

#[test]
fn test_stranger_cannot_terminate_early() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION);
    });

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().terminate_early(&token_id, &ctx.stranger),
        "caller is not the active renter"
    );
}

#[test]
fn test_cannot_rent_duration_exceeding_max() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().rent(&token_id, &ctx.renter, &(MAX_DURATION + 1)),
        "invalid rental duration"
    );
}

#[test]
fn test_cannot_rent_already_rented_nft() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION);
    });

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().rent(&token_id, &ctx.stranger, &RENTAL_DURATION),
        "nft is currently rented"
    );
}

#[test]
fn test_reclaim_before_expiry_reverts() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION);
    });

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().reclaim_expired(&token_id),
        "rental has not expired yet"
    );
}

#[test]
fn test_delist_prevents_new_rentals() {
    let ctx = Ctx::setup();
    let token_id = ctx.mint_and_list();

    ctx.env.with_mock_all_auths(|| {
        ctx.client().delist(&token_id, &ctx.owner);
    });

    let listing = ctx.client().get_listing(&token_id);
    assert_eq!(listing.is_listed, false);

    ctx.env.mock_all_auths();
    assert_reverts!(
        ctx.client().rent(&token_id, &ctx.renter, &RENTAL_DURATION),
        "nft is not listed for rent"
    );
}
