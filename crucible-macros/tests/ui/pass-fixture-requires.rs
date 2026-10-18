// A fixture may compose others through `requires`, and `setup_deps()` builds
// them in declaration order.
use crucible::prelude::*;

#[fixture]
pub struct TokenFixture {
    pub label: &'static str,
}

impl TokenFixture {
    pub fn setup() -> Self {
        Self { label: "token" }
    }
}

#[fixture]
pub struct OracleFixture {
    pub label: &'static str,
}

impl OracleFixture {
    pub fn setup() -> Self {
        Self { label: "oracle" }
    }
}

#[fixture(requires = [TokenFixture, OracleFixture])]
pub struct DexFixture {
    pub token: TokenFixture,
    pub oracle: OracleFixture,
}

impl DexFixture {
    pub fn setup() -> Self {
        let (token, oracle) = Self::setup_deps();
        Self { token, oracle }
    }
}

fn main() {
    let dex = DexFixture::setup();
    assert_eq!(dex.token.label, "token");
    assert_eq!(dex.oracle.label, "oracle");

    assert_eq!(DexFixture::DEPENDENCY_COUNT, 2);
    assert_eq!(
        <DexFixture as FixtureDeps>::DEPENDENCY_NAMES,
        ["TokenFixture", "OracleFixture"]
    );

    // A fixture without `requires` still participates in the graph.
    assert_eq!(TokenFixture::DEPENDENCY_COUNT, 0);
    assert!(<TokenFixture as FixtureDeps>::DEPENDENCY_NAMES.is_empty());
}
