// Naming the same fixture twice in `requires` is a mistake, not a request for
// two instances.
use crucible::prelude::*;

#[fixture]
pub struct TokenFixture {
    pub value: i32,
}

impl TokenFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

#[fixture(requires = [TokenFixture, TokenFixture])]
pub struct DexFixture {
    pub token: TokenFixture,
}

impl DexFixture {
    pub fn setup() -> Self {
        let (token, _) = Self::setup_deps();
        Self { token }
    }
}

fn main() {}
