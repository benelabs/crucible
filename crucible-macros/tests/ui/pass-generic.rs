// Generic struct — the macro must preserve the generic parameters on the impl block.
use crucible::prelude::*;

#[fixture]
pub struct WrapperFixture<T: Default + std::fmt::Debug + Clone> {
    pub inner: T,
}

impl<T: Default + std::fmt::Debug + Clone> WrapperFixture<T> {
    pub fn setup() -> Self {
        Self { inner: T::default() }
    }
}

fn main() {
    let mut f = WrapperFixture::<u32>::setup();
    assert_eq!(f.inner, 0);
    f.inner = 7;
    f.reset();
    assert_eq!(f.inner, 0);
    let _ = format!("{:?}", f);
}
