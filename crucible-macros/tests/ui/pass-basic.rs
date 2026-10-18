// Basic fixture with setup() — must compile and the generated reset() must work.
use crucible::prelude::*;

#[fixture]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 42 }
    }
}

fn main() {
    let mut f = MyFixture::setup();
    assert_eq!(f.value, 42);

    f.value = 100;
    f.reset();
    assert_eq!(f.value, 42);

    // Debug must have been derived automatically.
    let _ = format!("{:?}", f);
}
