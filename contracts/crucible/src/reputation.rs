use soroban_sdk::testutils::ContractFunctionSet;
use soroban_sdk::{contracttype, symbol_short, Address, Env, TryFromVal, TryIntoVal, Val};

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Reputation(Address),
}

pub struct ReputationContract;

impl Default for ReputationContract {
    fn default() -> Self {
        Self
    }
}

impl ReputationContract {
    fn initialize(&self, env: Env, admin: Address) {
        let existing: Option<Address> = env.storage().instance().get(&DataKey::Admin);
        if existing.is_some() {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.events().publish((symbol_short!("init"), admin), 0u32);
    }

    fn set_reputation(&self, env: Env, caller: Address, account: Address, score: i32) {
        caller.require_auth();
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        assert_eq!(caller, admin, "not admin");
        env.storage()
            .instance()
            .set(&DataKey::Reputation(account.clone()), &score);
        env.events()
            .publish((symbol_short!("rep_set"), account), score);
    }

    fn increase_reputation(&self, env: Env, caller: Address, account: Address, amount: i32) {
        caller.require_auth();
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        assert_eq!(caller, admin, "not admin");
        let current: i32 = env
            .storage()
            .instance()
            .get(&DataKey::Reputation(account.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Reputation(account.clone()), &(current + amount));
        env.events()
            .publish((symbol_short!("rep_inc"), account), amount);
    }

    fn decrease_reputation(&self, env: Env, caller: Address, account: Address, amount: i32) {
        caller.require_auth();
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        assert_eq!(caller, admin, "not admin");
        let current: i32 = env
            .storage()
            .instance()
            .get(&DataKey::Reputation(account.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Reputation(account.clone()), &(current - amount));
        env.events()
            .publish((symbol_short!("rep_dec"), account), amount);
    }

    fn get_reputation(&self, env: Env, account: Address) -> i32 {
        env.storage()
            .instance()
            .get(&DataKey::Reputation(account))
            .unwrap_or(0)
    }
}

impl ContractFunctionSet for ReputationContract {
    fn call(&self, func: &str, env: Env, args: &[Val]) -> Option<Val> {
        let addr = |i: usize| -> Option<Address> { Address::try_from_val(&env, args.get(i)?).ok() };
        let int = |i: usize| -> Option<i32> { i32::try_from_val(&env, args.get(i)?).ok() };

        match func {
            "initialize" => {
                let a = addr(0)?;
                self.initialize(env, a);
                Some(Val::from(()))
            }
            "set_rep" => {
                let (a, b, c) = (addr(0)?, addr(1)?, int(2)?);
                self.set_reputation(env, a, b, c);
                Some(Val::from(()))
            }
            "inc_rep" => {
                let (a, b, c) = (addr(0)?, addr(1)?, int(2)?);
                self.increase_reputation(env, a, b, c);
                Some(Val::from(()))
            }
            "dec_rep" => {
                let (a, b, c) = (addr(0)?, addr(1)?, int(2)?);
                self.decrease_reputation(env, a, b, c);
                Some(Val::from(()))
            }
            "get_rep" => {
                let a = addr(0)?;
                let score = self.get_reputation(env.clone(), a);
                score.try_into_val(&env).ok()
            }
            _ => None,
        }
    }
}

/// Client wrapper for `ReputationContract` in tests.
#[derive(Clone)]
pub struct ReputationContractClient {
    env: Env,
    address: Address,
}

impl ReputationContractClient {
    pub fn new(env: &Env, address: &Address) -> Self {
        Self {
            env: env.clone(),
            address: address.clone(),
        }
    }

    pub fn address(&self) -> &Address {
        &self.address
    }

    pub fn initialize(&self, admin: &Address) {
        let args: soroban_sdk::Vec<Val> = (admin,).try_into_val(&self.env).unwrap();
        self.env
            .invoke_contract::<Val>(&self.address, &symbol_short!("init"), args);
    }

    pub fn set_reputation(&self, admin: &Address, account: &Address, score: i32) {
        let args: soroban_sdk::Vec<Val> = (admin, account, score).try_into_val(&self.env).unwrap();
        self.env
            .invoke_contract::<Val>(&self.address, &symbol_short!("set_rep"), args);
    }

    pub fn increase_reputation(&self, admin: &Address, account: &Address, amount: i32) {
        let args: soroban_sdk::Vec<Val> = (admin, account, amount).try_into_val(&self.env).unwrap();
        self.env
            .invoke_contract::<Val>(&self.address, &symbol_short!("inc_rep"), args);
    }

    pub fn decrease_reputation(&self, admin: &Address, account: &Address, amount: i32) {
        let args: soroban_sdk::Vec<Val> = (admin, account, amount).try_into_val(&self.env).unwrap();
        self.env
            .invoke_contract::<Val>(&self.address, &symbol_short!("dec_rep"), args);
    }

    pub fn get_reputation(&self, account: &Address) -> i32 {
        let args: soroban_sdk::Vec<Val> = (account,).try_into_val(&self.env).unwrap();
        self.env
            .invoke_contract::<i32>(&self.address, &symbol_short!("get_rep"), args)
    }
}

#[cfg(test)]
impl ReputationContractClient {
    /// Test-only helper that mocks all authorizations before running `f`.
    ///
    /// Use this in tests that exercise happy-path contract behavior. For
    /// authorization tests, prefer `MockEnv::mock_auths` with specific entries
    /// so missing or invalid auth is not masked.
    pub fn with_mock_all_auths<R>(&self, f: impl FnOnce(&Self) -> R) -> R {
        self.env.mock_all_auths();
        f(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[test]
    fn test_reputation_get_unset_returns_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let address = env.register(ReputationContract, ());
        env.as_contract(&address, || {
            ReputationContract.initialize(env.clone(), admin.clone());
            assert_eq!(
                ReputationContract.get_reputation(env.clone(), user.clone()),
                0
            );
        });
    }
}
