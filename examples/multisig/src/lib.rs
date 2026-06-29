#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Bytes, Env, Vec};

#[contracttype]
#[derive(Clone)]
pub struct Proposal {
    pub proposer: Address,
    pub tx: Bytes,
    pub approvals: Vec<Address>,
    pub executed: bool,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owners,
    Threshold,
    NextId,
    Proposal(u64),
}

#[contract]
#[derive(Default)]
pub struct MultiSig;

#[contractimpl]
impl MultiSig {
    pub fn initialize(env: Env, owners: Vec<Address>, threshold: u32) {
        if env.storage().instance().has(&DataKey::Owners) {
            panic!("already initialized");
        }
        if owners.is_empty() || threshold == 0 || (threshold as usize) > owners.len() {
            panic!("invalid owners/threshold");
        }
        env.storage().instance().set(&DataKey::Owners, &owners);
        env.storage().instance().set(&DataKey::Threshold, &threshold);
        env.storage().instance().set(&DataKey::NextId, &1u64);
        env.events().publish((symbol_short!("initialized"),), ());
    }

    pub fn propose(env: Env, proposer: Address, tx: Bytes) -> u64 {
        // proposer must be an owner
        let owners: Vec<Address> = env.storage().instance().get(&DataKey::Owners).unwrap();
        if !owners.iter().any(|a| a == &proposer) {
            panic!("only owner can propose");
        }
        proposer.require_auth();

        let mut id: u64 = 1;
        if env.storage().instance().has(&DataKey::NextId) {
            id = env.storage().instance().get(&DataKey::NextId).unwrap();
        }

        let proposal = Proposal {
            proposer: proposer.clone(),
            tx: tx.clone(),
            approvals: Vec::new(&env),
            executed: false,
        };

        env.storage().instance().set(&DataKey::Proposal(id), &proposal);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        env.events().publish((symbol_short!("proposed"),), id);
        id
    }

    pub fn approve(env: Env, approver: Address, id: u64) {
        let owners: Vec<Address> = env.storage().instance().get(&DataKey::Owners).unwrap();
        if !owners.iter().any(|a| a == &approver) {
            panic!("only owner can approve");
        }
        approver.require_auth();

        let mut p: Proposal = env.storage().instance().get(&DataKey::Proposal(id)).unwrap();
        if p.executed {
            panic!("proposal already executed");
        }
        // check for duplicate approval
        if p.approvals.iter().any(|a| a == &approver) {
            return; // idempotent
        }
        p.approvals.push_back(approver.clone());
        env.storage().instance().set(&DataKey::Proposal(id), &p);
        env.events().publish((symbol_short!("approved"),), (id, approver));
    }

    pub fn execute(env: Env, executor: Address, id: u64) {
        let owners: Vec<Address> = env.storage().instance().get(&DataKey::Owners).unwrap();
        if !owners.iter().any(|a| a == &executor) {
            panic!("only owner can execute");
        }
        executor.require_auth();

        let threshold: u32 = env.storage().instance().get(&DataKey::Threshold).unwrap();
        let mut p: Proposal = env.storage().instance().get(&DataKey::Proposal(id)).unwrap();
        if p.executed {
            panic!("proposal already executed");
        }
        let approvals = p.approvals.len();
        if (approvals as u32) < threshold {
            panic!("not enough approvals");
        }
        p.executed = true;
        env.storage().instance().set(&DataKey::Proposal(id), &p);
        env.events().publish((symbol_short!("executed"),), id);
    }

    pub fn get_proposal(env: Env, id: u64) -> Proposal {
        env.storage().instance().get(&DataKey::Proposal(id)).unwrap()
    }
}

#[cfg(test)]
mod test;
