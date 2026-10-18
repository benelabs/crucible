//! Integration examples for the `#[crucible::quickcheck]` property-fuzzing macro.
//!
//! These exercise the arithmetic and boundary invariants that Soroban contracts
//! most often get wrong: overflow on `i128` token amounts, negative amounts
//! slipping past a check, and unauthorized privilege escalation.

use crucible::prelude::*;
use soroban_sdk::{contract, contracterror, contractimpl, Address, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum LedgerError {
    /// The caller is not the ledger's admin.
    Unauthorized = 1,
    /// A negative amount was supplied where a non-negative one was required.
    NegativeAmount = 2,
    /// The credit would overflow the account's balance.
    Overflow = 3,
    /// The account does not hold enough to cover the debit.
    Insufficient = 4,
}

/// A deliberately small balance ledger, written the way a real contract would
/// be: checked arithmetic, explicit sign checks, and an admin gate.
#[contract]
#[derive(Default)]
pub struct Ledger;

#[contractimpl]
impl Ledger {
    /// Credits `amount` to `balance`, rejecting negatives and overflow.
    pub fn credit(_env: Env, balance: i128, amount: i128) -> Result<i128, LedgerError> {
        if amount < 0 {
            return Err(LedgerError::NegativeAmount);
        }
        balance.checked_add(amount).ok_or(LedgerError::Overflow)
    }

    /// Debits `amount` from `balance`, rejecting negatives and overdrafts.
    pub fn debit(_env: Env, balance: i128, amount: i128) -> Result<i128, LedgerError> {
        if amount < 0 {
            return Err(LedgerError::NegativeAmount);
        }
        if amount > balance {
            return Err(LedgerError::Insufficient);
        }
        Ok(balance - amount)
    }

    /// An admin-gated operation, used to check that the gate cannot be bypassed.
    pub fn admin_op(_env: Env, admin: Address, caller: Address) -> Result<(), LedgerError> {
        if admin != caller {
            return Err(LedgerError::Unauthorized);
        }
        Ok(())
    }
}

fn ledger_env() -> MockEnv {
    // Property tests build one environment per generated case, so snapshot
    // capture is switched off to avoid writing hundreds of JSON files per run.
    MockEnv::builder()
        .without_snapshots()
        .with_contract::<Ledger>()
        .build()
}

fn ledger(env: &MockEnv) -> LedgerClient<'_> {
    LedgerClient::new(env.inner(), &env.contract_id::<Ledger>())
}

// ── Arithmetic invariants ────────────────────────────────────────────────────

/// Crediting then debiting the same amount is the identity.
#[crucible::quickcheck(cases = 64)]
fn credit_then_debit_is_the_identity(balance: SorobanAmount, amount: SorobanAmount) {
    // Keep both operands well inside the range so the round trip cannot
    // legitimately overflow; overflow itself is covered separately below.
    let balance = balance.get() % 1_000_000_000;
    let amount = amount.get() % 1_000_000_000;

    let env = ledger_env();
    let client = ledger(&env);

    let credited = client.credit(&balance, &amount);
    assert_eq!(credited, balance + amount);
    assert_eq!(client.debit(&credited, &amount), balance);
}

/// Crediting a non-negative amount never decreases the balance.
#[crucible::quickcheck(cases = 64)]
fn crediting_never_decreases_the_balance(balance: SorobanAmount, amount: SorobanAmount) {
    let balance = balance.get() % 1_000_000_000;
    let amount = amount.get() % 1_000_000_000;

    let env = ledger_env();
    let client = ledger(&env);

    assert!(client.credit(&balance, &amount) >= balance);
}

/// Debiting a non-negative amount never increases the balance.
#[crucible::quickcheck(cases = 64)]
fn debiting_never_increases_the_balance(balance: SorobanAmount, amount: SorobanAmount) {
    let balance = balance.get() % 1_000_000_000;
    let amount = (amount.get() % 1_000_000_000).min(balance);

    let env = ledger_env();
    let client = ledger(&env);

    assert!(client.debit(&balance, &amount) <= balance);
}

// ── Boundary invariants ──────────────────────────────────────────────────────

/// Any credit that would exceed `i128::MAX` is rejected rather than wrapping.
#[crucible::quickcheck(cases = 64)]
fn credit_overflow_is_rejected_not_wrapped(headroom: SorobanAmount) {
    // Pick a balance close enough to the ceiling that the credit must overflow.
    let headroom = headroom.get() % 1_000;
    let balance = i128::MAX - headroom;
    let amount = headroom + 1;

    let env = ledger_env();
    let client = ledger(&env);

    env.expect_revert(|| client.credit(&balance, &amount))
        .with_error(LedgerError::Overflow)
        .verify();
}

/// A credit that exactly reaches `i128::MAX` is still accepted.
#[crucible::quickcheck(cases = 64)]
fn a_credit_landing_exactly_on_the_ceiling_succeeds(headroom: SorobanAmount) {
    let headroom = headroom.get() % 1_000;
    let balance = i128::MAX - headroom;

    let env = ledger_env();
    let client = ledger(&env);

    assert_eq!(client.credit(&balance, &headroom), i128::MAX);
}

/// Debiting more than the balance is rejected rather than going negative.
#[crucible::quickcheck(cases = 64)]
fn overdrafts_are_rejected(balance: SorobanAmount, excess: SorobanAmount) {
    let balance = balance.get() % 1_000_000;
    let amount = balance + 1 + (excess.get() % 1_000);

    let env = ledger_env();
    let client = ledger(&env);

    env.expect_revert(|| client.debit(&balance, &amount))
        .with_error(LedgerError::Insufficient)
        .verify();
}

/// Negative amounts are rejected on both entry points, for every negative value.
#[crucible::quickcheck(cases = 64)]
fn negative_amounts_are_always_rejected(amount: SorobanI128) {
    // Map the whole signed range onto strictly negative values. `MIN` has no
    // representable negation, so it is used as-is.
    let amount = match amount.get() {
        i128::MIN => i128::MIN,
        value if value > 0 => -value,
        0 => -1,
        value => value,
    };

    let env = ledger_env();
    let client = ledger(&env);

    env.expect_revert(|| client.credit(&1_000_i128, &amount))
        .with_error(LedgerError::NegativeAmount)
        .verify();

    env.expect_revert(|| client.debit(&1_000_i128, &amount))
        .with_error(LedgerError::NegativeAmount)
        .verify();
}

// ── Privilege invariants ─────────────────────────────────────────────────────

/// A non-admin caller can never pass the admin gate, whichever accounts are used.
#[crucible::quickcheck(cases = 32)]
fn the_admin_gate_cannot_be_escalated(caller_index: SorobanU32) {
    let env = ledger_env();
    let client = ledger(&env);

    let admin = AccountBuilder::new(&env).name("admin").build();
    let intruder = AccountBuilder::new(&env)
        .name(&format!("intruder{}", caller_index.get() % 16))
        .build();

    // The admin itself always passes.
    client.admin_op(&admin.address(), &admin.address());

    // Anyone else is always rejected.
    env.expect_revert(|| client.admin_op(&admin.address(), &intruder.address()))
        .with_error(LedgerError::Unauthorized)
        .verify();
}

// ── Harness behaviour ────────────────────────────────────────────────────────

/// A fixed seed makes a property fully reproducible.
#[crucible::quickcheck(cases = 16, seed = 12345)]
fn a_seeded_property_is_deterministic(value: u32) {
    // Trivially true; the point is that the macro accepts `seed` and the run
    // is reproducible, which the runtime's own tests assert in detail.
    assert_eq!(value, value);
}

/// Tuple destructuring in the parameter list works as written.
#[crucible::quickcheck(cases = 16)]
fn parameters_may_be_destructured((low, high): (u8, u8)) {
    let (low, high) = if low <= high { (low, high) } else { (high, low) };
    assert!(low <= high);
}

/// The checkpoint engine and the property harness compose: each generated case
/// can speculatively mutate the ledger and roll it back.
#[crucible::quickcheck(cases = 32)]
fn speculative_writes_do_not_leak_between_cases(amount: SorobanAmount) {
    let amount = amount.get() % 1_000;

    let env = ledger_env();
    let client = ledger(&env);

    let before = env.checkpoint();
    let speculative = env.speculate(|| client.credit(&0_i128, &amount));
    assert_eq!(speculative, amount);

    // `speculate` restored the ledger, so the checkpoint is still the tip.
    env.rollback_to(before);
    assert_eq!(env.checkpoint_depth(), 1);
}
