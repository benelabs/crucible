// Struct that already derives Debug — the macro must not add a duplicate derive.
use crucible::prelude::*;

#[fixture]
#[derive(Debug, Clone)]
pub struct MyFixture {
    pub value: i32,
}

impl MyFixture {
    pub fn setup() -> Self {
        Self { value: 0 }
    }
}

fn main() {
    let mut f = MyFixture::setup();
    let _ = format!("{:?}", f);
    let _cloned = f.clone();
    f.reset();
    assert_eq!(f.value, 0);
}
