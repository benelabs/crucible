// Two fixtures that require each other form a cycle, which must be rejected at
// compile time rather than recursing forever at runtime.
use crucible::prelude::*;

#[fixture(requires = [BFixture])]
pub struct AFixture {
    pub value: i32,
}

impl AFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

#[fixture(requires = [AFixture])]
pub struct BFixture {
    pub value: i32,
}

impl BFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {
    let _ = AFixture::setup_deps();
}
