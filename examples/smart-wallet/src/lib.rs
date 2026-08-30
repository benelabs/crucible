// Location: examples/smart-wallet/src/lib.rs // Production requirement: Account Abstraction Smart Wallet with Social Recovery
#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Map, Vec,
};

/// Batched call instruction
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CallInstruction {
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
}

#[contracttype]
enum DataKey {
    Owner,
    Guardians,
    Threshold,
    DailyLimit,
    SpentToday,
    LastSpendTimestamp,
    RecoveryApprovals(Address),
}

#[contract]
#[derive(Default)]
pub struct SmartWallet;

#[contractimpl]
impl SmartWallet {
    /// Initialize the smart wallet with owner, guardians list, recovery threshold, and daily spending limit.
    pub fn initialize(
        env: Env,
        owner: Address,
        guardians: Vec<Address>,
        threshold: u32,
        daily_limit: i128,
    ) {
        if env.storage().instance().has(&DataKey::Owner) {
            panic!("already initialized");
        }
        if threshold == 0 || (threshold as u32) > (guardians.len() as u32) {
            panic!("invalid guardian threshold");
        }
        if daily_limit <= 0 {
            panic!("daily limit must be positive");
        }

        owner.require_auth();

        env.storage().instance().set(&DataKey::Owner, &owner);
        env.storage().instance().set(&DataKey::Guardians, &guardians);
        env.storage().instance().set(&DataKey::Threshold, &threshold);
        env.storage().instance().set(&DataKey::DailyLimit, &daily_limit);
        env.storage().instance().set(&DataKey::SpentToday, &0i128);
        env.storage().instance().set(&DataKey::LastSpendTimestamp, &env.ledger().timestamp());
    }

    /// Execute a single transfer enforcing the daily spending limit.
    pub fn execute_transfer(
        env: Env,
        caller: Address,
        token_addr: Address,
        recipient: Address,
        amount: i128,
    ) {
        let owner: Address = env
            .storage()
            .instance()
            .get(&DataKey::Owner)
            .expect("not initialized");

        if caller != owner {
            panic!("unauthorized caller");
        }
        caller.require_auth();

        if amount <= 0 {
            panic!("amount must be positive");
        }

        Self::check_and_update_daily_spending(&env, amount);

        token::TokenClient::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &recipient,
            &amount,
        );

        env.events().publish(
            (symbol_short!("transfer"), caller),
            (recipient, amount),
        );
    }

    /// Execute batched transfers in a single transaction.
    pub fn execute_batch(
        env: Env,
        caller: Address,
        calls: Vec<CallInstruction>,
    ) {
        let owner: Address = env
            .storage()
            .instance()
            .get(&DataKey::Owner)
            .expect("not initialized");

        if caller != owner {
            panic!("unauthorized caller");
        }
        caller.require_auth();

        if calls.is_empty() {
            panic!("empty batch calls");
        }

        let mut total_batch_amount: i128 = 0;
        for i in 0..calls.len() {
            let call = calls.get(i).unwrap();
            if call.amount <= 0 {
                panic!("call amount must be positive");
            }
            total_batch_amount = total_batch_amount
                .checked_add(call.amount)
                .unwrap_or_else(|| panic!("overflow"));
        }

        Self::check_and_update_daily_spending(&env, total_batch_amount);

        for i in 0..calls.len() {
            let call = calls.get(i).unwrap();
            token::TokenClient::new(&env, &call.token).transfer(
                &env.current_contract_address(),
                &call.recipient,
                &call.amount,
            );
        }

        env.events().publish((symbol_short!("batch"), caller), calls.len());
    }

    /// Guardian submits approval for social wallet recovery to a proposed new owner.
    pub fn approve_recovery(
        env: Env,
        guardian: Address,
        proposed_owner: Address,
    ) {
        let guardians: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Guardians)
            .expect("not initialized");

        let mut is_guardian = false;
        for i in 0..guardians.len() {
            if guardians.get(i).unwrap() == guardian {
                is_guardian = true;
                break;
            }
        }

        if !is_guardian {
            panic!("not an authorized guardian");
        }
        guardian.require_auth();

        let mut approvals: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::RecoveryApprovals(proposed_owner.clone()))
            .unwrap_or(Vec::new(&env));

        for i in 0..approvals.len() {
            if approvals.get(i).unwrap() == guardian {
                panic!("guardian already approved this recovery");
            }
        }

        approvals.push_back(guardian.clone());
        env.storage().instance().set(
            &DataKey::RecoveryApprovals(proposed_owner.clone()),
            &approvals,
        );

        let threshold: u32 = env.storage().instance().get(&DataKey::Threshold).unwrap();

        // Check if threshold quorum reached
        if approvals.len() >= threshold {
            let old_owner: Address = env.storage().instance().get(&DataKey::Owner).unwrap();
            env.storage().instance().set(&DataKey::Owner, &proposed_owner);

            env.events().publish(
                (symbol_short!("recovered"), proposed_owner),
                old_owner,
            );
        } else {
            env.events().publish(
                (symbol_short!("rec_vote"), proposed_owner),
                (guardian, approvals.len()),
            );
        }
    }

    fn check_and_update_daily_spending(env: &Env, amount: i128) {
        let daily_limit: i128 = env.storage().instance().get(&DataKey::DailyLimit).unwrap();
        let mut spent_today: i128 = env.storage().instance().get(&DataKey::SpentToday).unwrap();
        let last_timestamp: u64 = env.storage().instance().get(&DataKey::LastSpendTimestamp).unwrap();

        let current_time = env.ledger().timestamp();
        // 86,400 seconds in 1 day
        if current_time >= last_timestamp + 86_400 {
            spent_today = 0;
            env.storage().instance().set(&DataKey::LastSpendTimestamp, &current_time);
        }

        let new_spent = spent_today
            .checked_add(amount)
            .unwrap_or_else(|| panic!("overflow"));

        if new_spent > daily_limit {
            panic!("daily spending limit exceeded");
        }

        env.storage().instance().set(&DataKey::SpentToday, &new_spent);
    }

    /// Query current owner
    pub fn get_owner(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Owner)
            .expect("not initialized")
    }

    /// Query daily spending limits
    pub fn get_spending_status(env: Env) -> (i128, i128) {
        let daily_limit: i128 = env.storage().instance().get(&DataKey::DailyLimit).unwrap_or(0);
        let spent_today: i128 = env.storage().instance().get(&DataKey::SpentToday).unwrap_or(0);
        (spent_today, daily_limit)
    }
}

#[cfg(test)]
mod test;
