#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use soroban_sdk::{symbol_short, vec, Address, String, Vec};

use crate::{
    CraftedItem, CraftingRecipe, GamingCraftingEngine, GamingCraftingEngineClient,
    MaterialRequirement,
};

const RECIPE_ID: u32 = 101;
const BASE_POWER: u32 = 50;
const MAX_DURABILITY: u32 = 100;
const WOOD_REQ: i128 = 5;
const IRON_REQ: i128 = 2;

struct Ctx {
    pub env: MockEnv,
    pub id: Address,
    pub admin: AccountHandle,
    pub player: AccountHandle,
    pub wood_token: MockToken,
    pub iron_token: MockToken,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .at_timestamp(1_000_000)
            .with_contract::<GamingCraftingEngine>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("player", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<GamingCraftingEngine>();
        let admin = env.account("admin");
        let player = env.account("player");
        let wood_token = MockToken::new(&env, "WOOD", 0);
        let iron_token = MockToken::new(&env, "IRON", 0);

        wood_token.mint(&player, 100);
        iron_token.mint(&player, 50);

        Ctx {
            env,
            id,
            admin,
            player,
            wood_token,
            iron_token,
        }
    }

    fn client(&self) -> GamingCraftingEngineClient<'_> {
        GamingCraftingEngineClient::new(self.env.inner(), &self.id)
    }

    fn init_and_register_recipe(&self) {
        self.env.with_mock_all_auths(|| {
            self.client().initialize(&self.admin);

            let materials = vec![
                self.env.inner(),
                MaterialRequirement {
                    material_token: self.wood_token.address(),
                    amount: WOOD_REQ,
                },
                MaterialRequirement {
                    material_token: self.iron_token.address(),
                    amount: IRON_REQ,
                },
            ];

            self.client().register_recipe(
                &self.admin,
                &RECIPE_ID,
                &String::from_str(self.env.inner(), "Broadsword"),
                &materials,
                &BASE_POWER,
                &MAX_DURABILITY,
            );
        });
    }
}

#[test]
fn test_craft_item_burns_materials_and_mints_item() {
    let ctx = Ctx::setup();
    ctx.init_and_register_recipe();

    let initial_wood = ctx.wood_token.balance(&ctx.player);
    let initial_iron = ctx.iron_token.balance(&ctx.player);

    let item_id = ctx
        .env
        .with_mock_all_auths(|| ctx.client().craft_item(&ctx.player, &RECIPE_ID));

    assert_eq!(item_id, 1);
    assert_eq!(ctx.wood_token.balance(&ctx.player), initial_wood - WOOD_REQ);
    assert_eq!(ctx.iron_token.balance(&ctx.player), initial_iron - IRON_REQ);

    let item: CraftedItem = ctx.client().get_item(&item_id);
    assert_eq!(item.item_id, 1);
    assert_eq!(item.recipe_id, RECIPE_ID);
    assert_eq!(item.owner, ctx.player.address());
    assert!(item.power >= BASE_POWER);
    assert_eq!(item.current_durability, MAX_DURABILITY);
    assert_eq!(item.is_broken, false);
}

#[test]
fn test_durability_degradation_and_broken_rejection() {
    let ctx = Ctx::setup();
    ctx.init_and_register_recipe();

    let item_id = ctx
        .env
        .with_mock_all_auths(|| ctx.client().craft_item(&ctx.player, &RECIPE_ID));

    // Degradation on use
    let used_item = ctx
        .env
        .with_mock_all_auths(|| ctx.client().use_item(&ctx.player, &item_id, &40));
    assert_eq!(used_item.current_durability, 60);
    assert_eq!(used_item.is_broken, false);

    // Further degradation leading to breaking
    let broken_item = ctx
        .env
        .with_mock_all_auths(|| ctx.client().use_item(&ctx.player, &item_id, &60));
    assert_eq!(broken_item.current_durability, 0);
    assert_eq!(broken_item.is_broken, true);

    // Attempting to use a broken item must fail
    let res = ctx
        .env
        .with_mock_all_auths(|| ctx.client().try_use_item(&ctx.player, &item_id, &10));
    assert!(res.is_err());
}

#[test]
fn test_repair_broken_item() {
    let ctx = Ctx::setup();
    ctx.init_and_register_recipe();

    let item_id = ctx
        .env
        .with_mock_all_auths(|| ctx.client().craft_item(&ctx.player, &RECIPE_ID));

    // Break the item
    ctx.env
        .with_mock_all_auths(|| ctx.client().use_item(&ctx.player, &item_id, &100));

    let item_before = ctx.client().get_item(&item_id);
    assert_eq!(item_before.is_broken, true);

    // Repair with wood tokens
    ctx.env.with_mock_all_auths(|| {
        ctx.client().repair_item(
            &ctx.player,
            &item_id,
            &ctx.wood_token.address(),
            &10,
        );
    });

    let item_after = ctx.client().get_item(&item_id);
    assert_eq!(item_after.current_durability, MAX_DURABILITY);
    assert_eq!(item_after.is_broken, false);
}
