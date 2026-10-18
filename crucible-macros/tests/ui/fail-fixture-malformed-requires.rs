// Malformed requires syntax (unclosed bracket) must preserve the underlying parse failure.
// This validates Issue #1069: incomplete error propagation.
use crucible::prelude::*;

#[fixture(requires = [TokenFixture))]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
