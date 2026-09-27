#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    TokenAddress,
    FeeBps,
    TotalSupply,
    Balance(Address),
    FlashLoanInProgress,
}

pub const MAX_FEE_BPS: i128 = 10_000;
pub const FLASH_LOAN_CALLBACK_SUCCESS: [u8; 32] = [
    0x43, 0x52, 0x55, 0x43, 0x49, 0x42, 0x4c, 0x45, 0x5f, 0x46, 0x4c, 0x41, 0x53, 0x48, 0x5f,
    0x4d, 0x49, 0x4e, 0x54, 0x5f, 0x53, 0x55, 0x43, 0x43, 0x45, 0x53, 0x53, 0x5f, 0x56, 0x31,
    0x00, 0x01,
];

#[contractclient(name = "FlashBorrowerClient")]
pub trait FlashBorrower {
    fn on_flash_loan(
        env: Env,
        initiator: Address,
        token: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> BytesN<32>;
}

#[contract]
#[derive(Default)]
pub struct FlashMintToken;

#[contractimpl]
impl FlashMintToken {
    /// Initialize the Flash Mint contract.
    pub fn initialize(env: Env, admin: Address, fee_bps: i128) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        if !(0..=MAX_FEE_BPS).contains(&fee_bps) {
            panic!("fee_bps out of bounds");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        env.storage().instance().set(&DataKey::TotalSupply, &0i128);
    }

    /// Return the maximum flash loan amount available for a given token.
    /// EIP-3156: For flash minting, max loan is the maximum mintable supply (i128::MAX - current_supply).
    pub fn max_flash_loan(env: Env, token: Address) -> i128 {
        let current_contract = env.current_contract_address();
        if token != current_contract {
            return 0;
        }
        let total_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        i128::MAX.saturating_sub(total_supply)
    }

    /// Return the fee charged for a flash loan of `amount` tokens.
    /// EIP-3156: Revert if the token is not supported.
    pub fn flash_fee(env: Env, token: Address, amount: i128) -> i128 {
        let current_contract = env.current_contract_address();
        if token != current_contract {
            panic!("unsupported token");
        }
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let fee_bps: i128 = env.storage().instance().get(&DataKey::FeeBps).unwrap_or(0);
        (amount * fee_bps) / MAX_FEE_BPS
    }

    /// Execute an EIP-3156 flash mint loan.
    pub fn flash_loan(
        env: Env,
        receiver_address: Address,
        token: Address,
        amount: i128,
        data: Bytes,
    ) -> bool {
        let current_contract = env.current_contract_address();
        if token != current_contract {
            panic!("unsupported token");
        }
        if amount <= 0 {
            panic!("amount must be positive");
        }

        let max_loan = Self::max_flash_loan(env.clone(), token.clone());
        if amount > max_loan {
            panic!("amount exceeds max flash loan");
        }

        // Reentrancy guard check
        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::FlashLoanInProgress)
            .unwrap_or(false)
        {
            panic!("reentrancy detected");
        }
        env.storage()
            .instance()
            .set(&DataKey::FlashLoanInProgress, &true);

        let fee = Self::flash_fee(env.clone(), token.clone(), amount);
        let total_due = amount + fee;

        let total_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        // Flash mint: temporarily mint amount to receiver
        let receiver_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(receiver_address.clone()))
            .unwrap_or(0);
        env.storage().instance().set(
            &DataKey::Balance(receiver_address.clone()),
            &(receiver_bal + amount),
        );
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(total_supply + amount));

        env.events().publish(
            (symbol_short!("fl_mint"), receiver_address.clone()),
            amount,
        );

        // Invoke receiver callback
        let borrower = FlashBorrowerClient::new(&env, &receiver_address);
        let initiator = env.current_contract_address();
        let ret_hash = borrower.on_flash_loan(&initiator, &token, &amount, &fee, &data);

        let expected_hash = BytesN::from_array(&env, &FLASH_LOAN_CALLBACK_SUCCESS);
        if ret_hash != expected_hash {
            panic!("invalid flash loan callback response");
        }

        // Verify receiver has enough balance to burn (amount + fee)
        let updated_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(receiver_address.clone()))
            .unwrap_or(0);

        if updated_bal < total_due {
            panic!("insufficient balance to repay flash loan + fee");
        }

        // Burn minted amount + fee from receiver
        let new_bal = updated_bal - total_due;
        env.storage()
            .instance()
            .set(&DataKey::Balance(receiver_address.clone()), &new_bal);

        // New total supply decreases by amount (fee is kept as protocol revenue or burned)
        let updated_supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        let new_supply = updated_supply - total_due;
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &new_supply);

        env.storage()
            .instance()
            .set(&DataKey::FlashLoanInProgress, &false);

        env.events().publish(
            (symbol_short!("fl_burn"), receiver_address),
            total_due,
        );

        true
    }

    /// Mint tokens (admin only).
    pub fn mint(env: Env, to: Address, amount: i128) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);
        let supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::Balance(to.clone()), &(bal + amount));
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(supply + amount));
        env.events().publish((symbol_short!("mint"), to), amount);
    }

    /// Transfer tokens.
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic!("amount must be positive");
        }
        let from_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);
        if from_bal < amount {
            panic!("insufficient balance");
        }
        let to_bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::Balance(from.clone()), &(from_bal - amount));
        env.storage()
            .instance()
            .set(&DataKey::Balance(to.clone()), &(to_bal + amount));
        env.events()
            .publish((symbol_short!("xfer"), from, to), amount);
    }

    /// Get account balance.
    pub fn balance_of(env: Env, account: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(account))
            .unwrap_or(0)
    }

    /// Get total supply.
    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod test;
