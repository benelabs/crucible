#![no_std]
#![allow(deprecated)]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env};

/// Per-user reputation record.
#[contracttype]
#[derive(Clone)]
pub struct Reputation {
    /// Cumulative score (positive endorsements - negative flags).
    pub score: i32,
    /// Total number of endorsements received.
    pub endorsements: u32,
    /// Total number of flags received.
    pub flags: u32,
}

#[contracttype]
enum DataKey {
    Admin,
    /// Reputation(subject)
    Rep(Address),
    /// Whether an endorser has already endorsed a subject.
    Endorsed(Address, Address),
    /// Whether a flagger has already flagged a subject.
    Flagged(Address, Address),
}

/// On-chain reputation contract.
///
/// Any address may endorse or flag any other address once.
/// An admin may revoke (reset) a reputation record.
#[contract]
#[derive(Default)]
pub struct ReputationContract;

#[contractimpl]
impl ReputationContract {
    /// Initialise the contract and set the admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Endorse `subject`. Each caller may endorse a given subject at most once.
    pub fn endorse(env: Env, endorser: Address, subject: Address) {
        endorser.require_auth();
        if endorser == subject {
            panic!("cannot endorse yourself");
        }
        let key = DataKey::Endorsed(endorser.clone(), subject.clone());
        if env.storage().instance().has(&key) {
            panic!("already endorsed");
        }
        env.storage().instance().set(&key, &true);

        let mut rep = Self::get_or_default(&env, &subject);
        rep.score += 1;
        rep.endorsements += 1;
        env.storage()
            .instance()
            .set(&DataKey::Rep(subject.clone()), &rep);
        env.events()
            .publish((symbol_short!("endorse"),), (endorser, subject));
    }

    /// Flag `subject` negatively. Each caller may flag a given subject at most once.
    pub fn flag(env: Env, flagger: Address, subject: Address) {
        flagger.require_auth();
        if flagger == subject {
            panic!("cannot flag yourself");
        }
        let key = DataKey::Flagged(flagger.clone(), subject.clone());
        if env.storage().instance().has(&key) {
            panic!("already flagged");
        }
        env.storage().instance().set(&key, &true);

        let mut rep = Self::get_or_default(&env, &subject);
        rep.score -= 1;
        rep.flags += 1;
        env.storage()
            .instance()
            .set(&DataKey::Rep(subject.clone()), &rep);
        env.events()
            .publish((symbol_short!("flag"),), (flagger, subject));
    }

    /// Return the reputation for `subject`. Defaults to zero if no record exists.
    pub fn reputation(env: Env, subject: Address) -> Reputation {
        Self::get_or_default(&env, &subject)
    }

    /// Admin: reset a subject's reputation record.
    pub fn revoke(env: Env, subject: Address) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage()
            .instance()
            .remove(&DataKey::Rep(subject.clone()));
        env.events()
            .publish((symbol_short!("revoke"),), subject);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn get_or_default(env: &Env, subject: &Address) -> Reputation {
        env.storage()
            .instance()
            .get(&DataKey::Rep(subject.clone()))
            .unwrap_or(Reputation {
                score: 0,
                endorsements: 0,
                flags: 0,
            })
    }
}

#[cfg(test)]
mod test;
