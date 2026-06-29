// Applying #[fixture] to a function must not compile.
use crucible_macros::fixture;

#[fixture]
fn not_a_fixture() {}

fn main() {}
