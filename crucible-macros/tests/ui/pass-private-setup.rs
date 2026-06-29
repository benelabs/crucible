// A fixture with a private setup() in the same module must still compile and reset.
use crucible_macros::fixture;

#[fixture]
pub struct PrivateSetupFixture {
    pub value: i32,
}

impl PrivateSetupFixture {
    fn setup() -> Self {
        Self { value: 1 }
    }
}

fn main() {
    let mut f = PrivateSetupFixture::setup();
    assert_eq!(f.value, 1);
    f.value = 99;
    f.reset();
    assert_eq!(f.value, 1);
}
