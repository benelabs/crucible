// A fixture that lists itself as a dependency via path alias must fail.
// This validates Issue #1068: self-cycle detection with path normalization.
use crucible::prelude::*;

#[fixture(requires = [self::MyFixture])]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
