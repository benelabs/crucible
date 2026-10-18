// A fixture that names itself in `requires` is a cycle of length one.
use crucible::prelude::*;

#[fixture(requires = [SelfFixture])]
pub struct SelfFixture {
    pub value: i32,
}

impl SelfFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
