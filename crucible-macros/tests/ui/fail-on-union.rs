// Applying #[fixture] to a union must produce a clear compile error.
use crucible_macros::fixture;

#[fixture]
pub union MyFixture {
    pub a: u32,
    pub b: f32,
}

fn main() {}
