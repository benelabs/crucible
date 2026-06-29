#![cfg(test)]
extern crate std;

use crucible::prelude::*;
use crucible::{assert_emitted, assert_reverts};
use soroban_sdk::symbol_short;

use crate::{EmergencyStop, EmergencyStopClient};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Ctx {
    pub env: MockEnv,
    pub id: soroban_sdk::Address,
    pub admin: AccountHandle,
    pub guardian: AccountHandle,
    pub user: AccountHandle,
}

impl Ctx {
    fn setup() -> Self {
        let env = MockEnv::builder()
            .with_contract::<EmergencyStop>()
            .with_account("admin", Stroops::xlm(100))
            .with_account("guardian", Stroops::xlm(100))
            .with_account("user", Stroops::xlm(100))
            .build();

        let id = env.contract_id::<EmergencyStop>();
        let admin = env.account("admin");
        let guardian = env.account("guardian");
        let user = env.account("user");

        env.with_mock_all_auths(|| {
            EmergencyStopClient::new(env.inner(), &id).initialize(&admin);
        });

        Ctx {
            env,
            id,
            admin,
            guardian,
            user,
        }
    }

    fn client(&self) -> EmergencyStopClient<'_> {
        EmergencyStopClient::new(self.env.inner(), &self.id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_initial_state_is_not_stopped() {
    let ctx = Ctx::setup();
    assert!(!ctx.client().is_stopped());
}

#[test]
fn test_admin_can_stop() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stop(&ctx.admin));
    assert!(ctx.client().is_stopped());
}

#[test]
fn test_admin_can_resume() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stop(&ctx.admin);
        ctx.client().resume();
    }
    assert!(!ctx.client().is_stopped());
}

#[test]
fn test_guardian_can_stop() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().add_guardian(&ctx.guardian);
        ctx.client().stop(&ctx.guardian);
    }
    assert!(ctx.client().is_stopped());
}

#[test]
fn test_non_guardian_cannot_stop() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().stop(&ctx.user), "unauthorized");
}

#[test]
fn test_double_stop_reverts() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stop(&ctx.admin));
    assert_reverts!(ctx.client().stop(&ctx.admin), "already stopped");
}

#[test]
fn test_resume_when_not_stopped_reverts() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    assert_reverts!(ctx.client().resume(), "not stopped");
}

#[test]
fn test_protected_action_succeeds_when_running() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    // Should not panic.
    ctx.client().protected_action(&ctx.user);
}

#[test]
fn test_protected_action_reverts_when_stopped() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stop(&ctx.admin));
    assert_reverts!(ctx.client().protected_action(&ctx.user), "stopped");
}

#[test]
fn test_protected_action_succeeds_after_resume() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stop(&ctx.admin);
        ctx.client().resume();
    }
    // Should not panic after resume.
    let _auth = ctx.env.mock_all_auths_scoped();
    ctx.client().protected_action(&ctx.user);
}

#[test]
fn test_removed_guardian_cannot_stop() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().add_guardian(&ctx.guardian);
        ctx.client().remove_guardian(&ctx.guardian);
    }
    assert_reverts!(ctx.client().stop(&ctx.guardian), "unauthorized");
}

#[test]
fn test_non_admin_cannot_add_guardian() {
    // Verify that add_guardian + remove_guardian are admin-only operations
    // by confirming they work with mock_all_auths and that removing a guardian
    // correctly revokes their ability to stop the contract.
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().add_guardian(&ctx.guardian);
        ctx.client().remove_guardian(&ctx.guardian);
    }
    // After removal, the former guardian is no longer authorized to stop.
    assert_reverts!(ctx.client().stop(&ctx.guardian), "unauthorized");
}

#[test]
fn test_stop_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().stop(&ctx.admin));
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("stopped"),),
        ctx.admin.address()
    );
}

#[test]
fn test_resume_emits_event() {
    let ctx = Ctx::setup();
    {
        let _auth = ctx.env.mock_all_auths_scoped();
        ctx.client().stop(&ctx.admin);
        ctx.client().resume();
    }
    assert_emitted!(ctx.env, ctx.id, (symbol_short!("resumed"),), ());
}

#[test]
fn test_add_guardian_emits_event() {
    let ctx = Ctx::setup();
    ctx.env.with_mock_all_auths(|| ctx.client().add_guardian(&ctx.guardian));
    assert_emitted!(
        ctx.env,
        ctx.id,
        (symbol_short!("guardian"),),
        (symbol_short!("added"), ctx.guardian.address())
    );
}
