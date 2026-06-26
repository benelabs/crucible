#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::{symbol_short, Address};

use crate::{ReputationContract, ReputationContractClient};

// ---------------------------------------------------------------------------
// Test fixture
// ---------------------------------------------------------------------------

#[fixture]
struct Ctx {
    pub env: MockEnv,
    pub id: Address,
}

impl Ctx {
    pub fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<ReputationContract>()
            .build();
        let id = env.contract_id::<ReputationContract>();
        Ctx { env, id }
    }

    fn client(&self) -> ReputationContractClient<'_> {
        ReputationContractClient::new(self.env.inner(), &self.id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_initial_reputation_is_zero() {
    let f = Ctx::setup();
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&f.env.account("admin").address());
    assert_eq!(f.client().get_reputation(&user.address()), 0);
}

#[test]
fn test_set_reputation() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    f.client()
        .set_reputation(&admin.address(), &user.address(), &100);
    assert_eq!(f.client().get_reputation(&user.address()), 100);
}

#[test]
fn test_increase_reputation() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    f.client()
        .set_reputation(&admin.address(), &user.address(), &100);
    f.client()
        .increase_reputation(&admin.address(), &user.address(), &50);
    assert_eq!(f.client().get_reputation(&user.address()), 150);
}

#[test]
fn test_decrease_reputation() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    f.client()
        .set_reputation(&admin.address(), &user.address(), &100);
    f.client()
        .decrease_reputation(&admin.address(), &user.address(), &30);
    assert_eq!(f.client().get_reputation(&user.address()), 70);
}

#[test]
fn test_set_reputation_emits_event() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    f.client()
        .set_reputation(&admin.address(), &user.address(), &42);
    assert_emitted!(
        f.env,
        f.id,
        (symbol_short!("rep_set"), user.address()),
        42_i32
    );
}

#[test]
fn test_increase_reputation_emits_event() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    f.client()
        .increase_reputation(&admin.address(), &user.address(), &10);
    assert_emitted!(
        f.env,
        f.id,
        (symbol_short!("rep_inc"), user.address()),
        10_i32
    );
}

#[test]
fn test_decrease_reputation_emits_event() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    f.client()
        .decrease_reputation(&admin.address(), &user.address(), &5);
    assert_emitted!(
        f.env,
        f.id,
        (symbol_short!("rep_dec"), user.address()),
        5_i32
    );
}

#[test]
fn test_non_admin_cannot_set_reputation() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    let user = f.env.account("user");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    // user tries to set their own reputation
    assert_reverts!(
        f.client()
            .set_reputation(&user.address(), &user.address(), &999)
    );
}

#[test]
fn test_double_initialize_reverts() {
    let f = Ctx::setup();
    let admin = f.env.account("admin");
    f.env.mock_all_auths();
    f.client().initialize(&admin.address());
    assert_reverts!(f.client().initialize(&admin.address()), "already initialized");
}
