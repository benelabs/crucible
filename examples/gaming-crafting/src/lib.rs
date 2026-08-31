// Location: examples/gaming-crafting/src/lib.rs // Production requirement: Gaming Item Crafting & Durability Degradation Engine
#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env, Map, String,
    Vec,
};

/// Material requirement for crafting recipes.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRequirement {
    pub material_token: Address,
    pub amount: i128,
}

/// Crafting Recipe defining ingredients and output item base specs.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CraftingRecipe {
    pub recipe_id: u32,
    pub name: String,
    pub materials: Vec<MaterialRequirement>,
    pub base_power: u32,
    pub max_durability: u32,
    pub enabled: bool,
}

/// Crafted Game Item NFT with dynamic stats and degradation state.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CraftedItem {
    pub item_id: u64,
    pub recipe_id: u32,
    pub owner: Address,
    pub power: u32,
    pub max_durability: u32,
    pub current_durability: u32,
    pub is_broken: bool,
}

#[contracttype]
enum DataKey {
    Admin,
    Recipe(u32),
    Item(u64),
    NextItemId,
}

#[contract]
#[derive(Default)]
pub struct GamingCraftingEngine;

#[contractimpl]
impl GamingCraftingEngine {
    /// Initialize the crafting engine with an admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::NextItemId, &1u64);
    }

    /// Register or update a crafting recipe. Admin only.
    pub fn register_recipe(
        env: Env,
        admin: Address,
        recipe_id: u32,
        name: String,
        materials: Vec<MaterialRequirement>,
        base_power: u32,
        max_durability: u32,
    ) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        if admin != stored_admin {
            panic!("unauthorized admin");
        }
        admin.require_auth();

        if materials.is_empty() {
            panic!("recipe must contain materials");
        }
        if max_durability == 0 {
            panic!("max durability must be positive");
        }

        let recipe = CraftingRecipe {
            recipe_id,
            name,
            materials,
            base_power,
            max_durability,
            enabled: true,
        };

        env.storage().instance().set(&DataKey::Recipe(recipe_id), &recipe);
        env.events().publish((symbol_short!("recipe"), recipe_id), true);
    }

    /// Craft an item by burning/transferring required materials and minting NFT with randomized stat modifier.
    pub fn craft_item(env: Env, crafter: Address, recipe_id: u32) -> u64 {
        crafter.require_auth();

        let recipe: CraftingRecipe = env
            .storage()
            .instance()
            .get(&DataKey::Recipe(recipe_id))
            .expect("recipe not found");

        if !recipe.enabled {
            panic!("recipe is disabled");
        }

        // Burn/transfer materials from crafter to contract / burn
        for i in 0..recipe.materials.len() {
            let req = recipe.materials.get(i).unwrap();
            token::TokenClient::new(&env, &req.material_token).transfer(
                &crafter,
                &env.current_contract_address(),
                &req.amount,
            );
        }

        let item_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextItemId)
            .unwrap_or(1);
        env.storage().instance().set(&DataKey::NextItemId, &(item_id + 1));

        // Deterministic pseudo-random stat modifier based on ledger sequence, timestamp, item_id, and crafter
        let pseudo_random = ((env.ledger().sequence() as u64)
            ^ env.ledger().timestamp()
            ^ item_id)
            % 20; // 0..19 bonus stat modifier

        let final_power = recipe.base_power + (pseudo_random as u32);

        let item = CraftedItem {
            item_id,
            recipe_id,
            owner: crafter.clone(),
            power: final_power,
            max_durability: recipe.max_durability,
            current_durability: recipe.max_durability,
            is_broken: false,
        };

        env.storage().instance().set(&DataKey::Item(item_id), &item);
        env.events().publish(
            (symbol_short!("crafted"), item_id),
            (crafter, recipe_id, final_power),
        );

        item_id
    }

    /// Interact with an item (e.g. battle, gather), causing durability degradation.
    pub fn use_item(env: Env, caller: Address, item_id: u64, durability_cost: u32) -> CraftedItem {
        caller.require_auth();

        let mut item: CraftedItem = env
            .storage()
            .instance()
            .get(&DataKey::Item(item_id))
            .expect("item not found");

        if item.owner != caller {
            panic!("not item owner");
        }

        if item.is_broken || item.current_durability == 0 {
            panic!("item is broken and cannot be used");
        }

        if durability_cost == 0 {
            panic!("durability cost must be positive");
        }

        if item.current_durability <= durability_cost {
            item.current_durability = 0;
            item.is_broken = true;
        } else {
            item.current_durability -= durability_cost;
        }

        env.storage().instance().set(&DataKey::Item(item_id), &item);
        env.events().publish(
            (symbol_short!("item_used"), item_id),
            (item.current_durability, item.is_broken),
        );

        item
    }

    /// Repair a degraded or broken item restoring durability.
    pub fn repair_item(
        env: Env,
        caller: Address,
        item_id: u64,
        repair_token: Address,
        repair_cost: i128,
    ) {
        caller.require_auth();

        let mut item: CraftedItem = env
            .storage()
            .instance()
            .get(&DataKey::Item(item_id))
            .expect("item not found");

        if item.owner != caller {
            panic!("not item owner");
        }

        if repair_cost <= 0 {
            panic!("repair cost must be positive");
        }

        token::TokenClient::new(&env, &repair_token).transfer(
            &caller,
            &env.current_contract_address(),
            &repair_cost,
        );

        item.current_durability = item.max_durability;
        item.is_broken = false;

        env.storage().instance().set(&DataKey::Item(item_id), &item);
        env.events().publish(
            (symbol_short!("repaired"), item_id),
            item.current_durability,
        );
    }

    /// Query item details.
    pub fn get_item(env: Env, item_id: u64) -> CraftedItem {
        env.storage()
            .instance()
            .get(&DataKey::Item(item_id))
            .expect("item not found")
    }

    /// Query recipe details.
    pub fn get_recipe(env: Env, recipe_id: u32) -> CraftingRecipe {
        env.storage()
            .instance()
            .get(&DataKey::Recipe(recipe_id))
            .expect("recipe not found")
    }
}

#[cfg(test)]
mod test;
