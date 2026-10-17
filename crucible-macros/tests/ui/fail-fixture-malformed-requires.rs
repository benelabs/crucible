// Malformed requires syntax must produce a detailed error that preserves the
// underlying parse failure, not a generic message.
// This validates Issue #1069: incomplete error propagation.
use crucible_macros::fixture;

#[fixture(requires = TokenFixture)]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
