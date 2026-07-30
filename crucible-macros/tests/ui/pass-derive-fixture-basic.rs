use crucible_macros::Fixture;
use crucible::prelude::*;

#[derive(Fixture)]
struct DexFixture {
    env: MockEnv,
    #[contract_client(contract = AmmPool)]
    pool_client: AmmPoolClient,
}

fn main() {}
