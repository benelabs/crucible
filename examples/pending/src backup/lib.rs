#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

/// A meta-transaction (gasless) request signed by the user.
#[contracttype]
#[derive(Clone)]
pub struct MetaTx {
    /// The user whose tokens will be transferred.
    pub from: Address,
    /// Recipient of the transfer.
    pub to: Address,
    /// Token contract address.
    pub token: Address,
    /// Amount to transfer.
    pub amount: i128,
    /// Nonce to prevent replay attacks.
    pub nonce: u64,
    /// Deadline (unix timestamp) after which this meta-tx is invalid.
    pub deadline: u64,
}

#[contracttype]
enum DataKey {
    /// Admin / relayer address.
    Admin,
    /// Per-user nonce counter.
    Nonce(Address),
}

/// A gasless transaction (meta-transaction) contract.
///
/// A user signs a `MetaTx` off-chain. A trusted relayer submits it on-chain,
/// paying the network fee. The contract verifies the nonce and deadline, then
/// executes the token transfer on behalf of the user.
///
/// In Soroban, "signing" is handled by `require_auth` — the user's auth entry
/// is attached to the transaction by the relayer. This contract enforces:
/// - Nonce uniqueness (replay protection).
/// - Deadline enforcement (expiry protection).
/// - Relayer-only submission.
#[contract]
#[derive(Default)]
pub struct Gasless;

#[contractimpl]
impl Gasless {
    /// Initialize the contract with a trusted relayer address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Execute a meta-transaction on behalf of `meta_tx.from`.
    ///
    /// Must be called by the registered relayer (admin).
    /// The user's authorization is verified via `meta_tx.from.require_auth()`.
    pub fn execute(env: Env, relayer: Address, meta_tx: MetaTx) {
        // Only the registered relayer may submit.
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        if relayer != admin {
            panic!("unauthorized relayer");
        }
        relayer.require_auth();

        // Deadline check.
        let now = env.ledger().timestamp();
        if now > meta_tx.deadline {
            panic!("meta-tx expired");
        }

        // Nonce check — must match the stored next-nonce for this user.
        let expected_nonce: u64 = env
            .storage()
            .instance()
            .get(&DataKey::Nonce(meta_tx.from.clone()))
            .unwrap_or(0u64);
        if meta_tx.nonce != expected_nonce {
            panic!("invalid nonce");
        }

        // Require the user's authorization (attached by the relayer).
        meta_tx.from.require_auth();

        // Advance nonce.
        env.storage()
            .instance()
            .set(&DataKey::Nonce(meta_tx.from.clone()), &(expected_nonce + 1));

        // Execute the transfer.
        token::TokenClient::new(&env, &meta_tx.token).transfer(
            &meta_tx.from,
            &meta_tx.to,
            &meta_tx.amount,
        );

        env.events().publish(
            (symbol_short!("executed"),),
            (meta_tx.from, meta_tx.to, meta_tx.amount, meta_tx.nonce),
        );
    }

    /// Return the current nonce for `user` (the next expected nonce).
    pub fn nonce(env: Env, user: Address) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::Nonce(user))
            .unwrap_or(0u64)
    }

    /// Return the relayer address.
    pub fn relayer(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized")
    }
}

/// Helper to build a `MetaTx` value (used in tests).
pub fn make_meta_tx(
    _env: &Env,
    from: Address,
    to: Address,
    token: Address,
    amount: i128,
    nonce: u64,
    deadline: u64,
) -> MetaTx {
    MetaTx {
        from,
        to,
        token,
        amount,
        nonce,
        deadline,
    }
}

#[cfg(test)]
mod test;
