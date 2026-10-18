// Passing arguments to #[fixture] must produce a clear compile error.
use crucible_macros::fixture;

#[fixture(debug)]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {}
