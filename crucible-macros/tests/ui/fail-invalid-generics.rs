// A generic fixture whose setup() returns the wrong Self type must not compile.
use crucible_macros::fixture;

#[fixture]
pub struct GenericFixture<T> {
    pub value: T,
}

impl<T> GenericFixture<T> {
    pub fn setup() -> GenericFixture<u32> {
        GenericFixture { value: 0 }
    }
}

fn main() {
    let mut f = GenericFixture { value: 0u32 };
    f.reset();
}
