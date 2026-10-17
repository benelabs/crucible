// A fixture that lists duplicate dependencies (via path alias) must fail.
// This validates Issue #1068: path normalization for cycle detection.
use crucible_macros::fixture;

mod fixtures {
    pub struct TokenFixture;
}

#[fixture(requires = [TokenFixture, fixtures::TokenFixture])]
pub struct DexFixture {
    pub value: i32,
}

impl DexFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

struct TokenFixture;

fn main() {}
