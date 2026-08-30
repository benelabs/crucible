// #[derive(Fixture)] must wire a contract client from the generated MockEnv.
use crucible::prelude::*;
use crucible_macros::Fixture;
use soroban_sdk::{contract, contractimpl, Env};

#[contract]
#[derive(Default)]
pub struct AmmPool;

#[contractimpl]
impl AmmPool {
    pub fn ping(_env: Env) -> u32 {
        1
    }
}

// Soroban's generated clients do not implement `Debug`, so the fixture cannot
// derive it either.
#[derive(Fixture)]
struct DexFixture {
    env: MockEnv,
    #[contract_client(contract = AmmPool)]
    pool_client: AmmPoolClient<'static>,
}

fn main() {
    let fixture = DexFixture::setup();
    assert_eq!(fixture.pool_client.ping(), 1);
    let _ = &fixture.env;
}
