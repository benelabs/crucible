#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Vec,
};

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum OptionType {
    Call,
    Put,
}

#[contracttype]
#[derive(Clone, PartialEq, Debug)]
pub enum OptionStatus {
    Active,
    Exercised,
    Expired,
    Cancelled,
}

#[contracttype]
#[derive(Clone)]
pub struct OptionPosition {
    pub writer: Address,
    pub holder: Address,
    pub option_type: OptionType,
    pub strike_price: i128,
    pub premium: i128,
    pub quantity: i128,
    pub collateral: i128,
    pub expiry: u64,
    pub status: OptionStatus,
}

#[contracttype]
enum DataKey {
    State,
    Position(Address),
}

#[contract]
#[derive(Default)]
pub struct OptionsProtocol;

#[contractimpl]
impl OptionsProtocol {
    pub fn mint(
        env: Env,
        writer: Address,
        holder: Address,
        option_type: OptionType,
        strike_price: i128,
        premium: i128,
        quantity: i128,
        collateral: i128,
        expiry: u64,
    ) -> i128 {
        if expiry <= env.ledger().timestamp() {
            panic!("expiry must be in the future");
        }
        if strike_price <= 0 || premium < 0 || quantity <= 0 || collateral <= 0 {
            panic!("invalid option parameters");
        }
        if matches!(option_type, OptionType::Put) && collateral < strike_price * quantity {
            panic!("put collateral insufficient");
        }

        writer.require_auth();
        let token = env.storage().instance().get::<DataKey, Address>(&DataKey::State).unwrap();
        token::TokenClient::new(&env, &token).transfer(
            &writer,
            &env.current_contract_address(),
            &collateral,
        );

        let position = OptionPosition {
            writer: writer.clone(),
            holder: holder.clone(),
            option_type,
            strike_price,
            premium,
            quantity,
            collateral,
            expiry,
            status: OptionStatus::Active,
        };

        env.storage().instance().set(&DataKey::Position(holder.clone()), &position);
        env.events().publish((symbol_short!("mint"), holder), (strike_price, quantity, expiry));
        quantity
    }

    pub fn buy(env: Env, holder: Address, option_id: Address, amount: i128) {
        let mut position: OptionPosition = env.storage().instance().get(&DataKey::Position(option_id.clone())).unwrap();
        if position.status != OptionStatus::Active {
            panic!("option is not active");
        }
        if env.ledger().timestamp() >= position.expiry {
            panic!("option expired");
        }
        if amount <= 0 || amount > position.quantity {
            panic!("invalid amount");
        }
        holder.require_auth();

        let token = env.storage().instance().get::<DataKey, Address>(&DataKey::State).unwrap();
        token::TokenClient::new(&env, &token).transfer(
            &holder,
            &position.writer,
            &position.premium * amount,
        );

        position.quantity -= amount;
        if position.quantity == 0 {
            position.status = OptionStatus::Exercised;
        }

        env.storage().instance().set(&DataKey::Position(option_id), &position);
        env.events().publish((symbol_short!("buy"), holder), (option_id, amount));
    }

    pub fn exercise(env: Env, holder: Address) {
        let mut position: OptionPosition = env.storage().instance().get(&DataKey::Position(holder.clone())).unwrap();
        if position.status != OptionStatus::Active {
            panic!("option is not active");
        }
        if env.ledger().timestamp() >= position.expiry {
            panic!("option expired");
        }
        holder.require_auth();

        let token = env.storage().instance().get::<DataKey, Address>(&DataKey::State).unwrap();
        let spot = Self::oracle_price(&env);
        let intrinsic = match position.option_type {
            OptionType::Call => (spot - position.strike_price).max(0),
            OptionType::Put => (position.strike_price - spot).max(0),
        };

        if intrinsic <= 0 {
            position.status = OptionStatus::Expired;
            env.storage().instance().set(&DataKey::Position(holder), &position);
            return;
        }

        let payout = intrinsic * position.quantity;
        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &holder,
            &payout,
        );
        token::TokenClient::new(&env, &token).burn(&env.current_contract_address(), &position.collateral);
        position.status = OptionStatus::Exercised;
        env.storage().instance().set(&DataKey::Position(holder), &position);
        env.events().publish((symbol_short!("exercise"), holder), payout);
    }

    pub fn initialize(env: Env, admin: Address, token: Address) {
        if env.storage().instance().has(&DataKey::State) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::State, &token);
    }

    pub fn writer_unlock(env: Env, writer: Address) {
        let position: OptionPosition = env.storage().instance().get(&DataKey::Position(writer.clone())).unwrap();
        if position.status == OptionStatus::Active && env.ledger().timestamp() < position.expiry {
            panic!("option has not expired");
        }
        writer.require_auth();
        let token = env.storage().instance().get::<DataKey, Address>(&DataKey::State).unwrap();
        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &writer,
            &position.collateral,
        );
        env.events().publish((symbol_short!("unlock"), writer), position.collateral);
    }

    fn oracle_price(env: &Env) -> i128 {
        1000
    }
}

#[cfg(test)]
mod test;
