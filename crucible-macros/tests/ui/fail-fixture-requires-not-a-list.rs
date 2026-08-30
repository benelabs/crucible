// `requires` expects a list of fixture types.
use crucible::prelude::*;

#[fixture(requires = TokenFixture)]
pub struct DexFixture {
    pub value: i32,
}

impl DexFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
