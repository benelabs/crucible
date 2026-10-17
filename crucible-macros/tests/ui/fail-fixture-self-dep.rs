// A fixture that lists itself as a dependency must fail.
// This validates Issue #1068: self-cycle detection.
use crucible_macros::fixture;

#[fixture(requires = [MyFixture])]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
