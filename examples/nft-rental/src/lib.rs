// Location: examples/nft-rental/src/lib.rs // Production requirement: Decentralized Peer-to-Peer Rental Protocol with Collateral Escrow
#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, String,
};

/// NFT asset record retaining underlying immutable ownership.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct NFT {
    pub token_id: u64,
    pub owner: Address,
    pub metadata_uri: String,
}

/// Rental listing terms set by the asset owner.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RentalListing {
    pub token_id: u64,
    pub owner: Address,
    pub payment_token: Address,
    pub rental_fee: i128,
    pub collateral_amount: i128,
    pub max_duration: u64,
    pub is_listed: bool,
}

/// Active or historical rental agreement.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RentalAgreement {
    pub token_id: u64,
    pub renter: Address,
    pub owner: Address,
    pub payment_token: Address,
    pub start_time: u64,
    pub duration: u64,
    pub expires_at: u64,
    pub rental_fee_paid: i128,
    pub collateral_escrowed: i128,
    pub is_active: bool,
}

#[contracttype]
enum DataKey {
    NFT(u64),
    Listing(u64),
    Agreement(u64),
    NextTokenId,
}

/// Decentralized Peer-to-Peer Rental Protocol with Collateral Escrow.
///
/// Features:
/// - Distinct separation of Owner rights from User (usability) rights.
/// - Automatic reclamation of usability rights upon expiration of rental duration.
/// - Collateral escrow held securely during active leasing periods.
/// - Early rental termination with instant collateral refund to renter.
/// - Full lifecycle event emissions.
#[contract]
#[derive(Default)]
pub struct NFTRentalProtocol;

#[contractimpl]
impl NFTRentalProtocol {
    /// Mint a new NFT to the designated owner.
    pub fn mint(env: Env, owner: Address, metadata_uri: String) -> u64 {
        let next_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextTokenId)
            .unwrap_or(1);

        let nft = NFT {
            token_id: next_id,
            owner: owner.clone(),
            metadata_uri,
        };

        env.storage().instance().set(&DataKey::NFT(next_id), &nft);
        env.storage().instance().set(&DataKey::NextTokenId, &(next_id + 1));

        env.events().publish((symbol_short!("mint"), next_id), owner);
        next_id
    }

    /// List an NFT for peer-to-peer rental with collateral escrow requirements.
    pub fn list_for_rent(
        env: Env,
        token_id: u64,
        owner: Address,
        payment_token: Address,
        rental_fee: i128,
        collateral_amount: i128,
        max_duration: u64,
    ) {
        owner.require_auth();

        let nft: NFT = env
            .storage()
            .instance()
            .get(&DataKey::NFT(token_id))
            .unwrap_or_else(|| panic!("nft not found"));

        if nft.owner != owner {
            panic!("caller is not nft owner");
        }
        if rental_fee < 0 {
            panic!("rental fee cannot be negative");
        }
        if collateral_amount < 0 {
            panic!("collateral cannot be negative");
        }
        if max_duration == 0 {
            panic!("max duration must be positive");
        }

        if Self::is_rented(env.clone(), token_id) {
            panic!("cannot list actively rented nft");
        }

        let listing = RentalListing {
            token_id,
            owner: owner.clone(),
            payment_token,
            rental_fee,
            collateral_amount,
            max_duration,
            is_listed: true,
        };

        env.storage().instance().set(&DataKey::Listing(token_id), &listing);
        env.events().publish(
            (symbol_short!("list"), token_id),
            (rental_fee, collateral_amount),
        );
    }

    /// Delist an NFT from rental marketplace.
    pub fn delist(env: Env, token_id: u64, owner: Address) {
        owner.require_auth();

        let mut listing = Self::get_listing(env.clone(), token_id);
        if listing.owner != owner {
            panic!("caller is not listing owner");
        }
        if Self::is_rented(env.clone(), token_id) {
            panic!("cannot delist actively rented nft");
        }

        listing.is_listed = false;
        env.storage().instance().set(&DataKey::Listing(token_id), &listing);
        env.events().publish((symbol_short!("delist"), token_id), owner);
    }

    /// Rent an NFT, escrowing collateral in contract and paying rental fee to owner.
    pub fn rent(env: Env, token_id: u64, renter: Address, duration: u64) {
        renter.require_auth();

        let listing = Self::get_listing(env.clone(), token_id);
        if !listing.is_listed {
            panic!("nft is not listed for rent");
        }
        if duration == 0 || duration > listing.max_duration {
            panic!("invalid rental duration");
        }
        if Self::is_rented(env.clone(), token_id) {
            panic!("nft is currently rented");
        }

        let total_required = listing
            .rental_fee
            .checked_add(listing.collateral_amount)
            .unwrap_or_else(|| panic!("amount overflow"));

        // Transfer rental fee + collateral from renter to contract
        if total_required > 0 {
            token::TokenClient::new(&env, &listing.payment_token).transfer(
                &renter,
                &env.current_contract_address(),
                &total_required,
            );
        }

        // Payout rental fee directly to owner
        if listing.rental_fee > 0 {
            token::TokenClient::new(&env, &listing.payment_token).transfer(
                &env.current_contract_address(),
                &listing.owner,
                &listing.rental_fee,
            );
        }

        let now = env.ledger().timestamp();
        let expires_at = now
            .checked_add(duration)
            .unwrap_or_else(|| panic!("timestamp overflow"));

        let agreement = RentalAgreement {
            token_id,
            renter: renter.clone(),
            owner: listing.owner.clone(),
            payment_token: listing.payment_token,
            start_time: now,
            duration,
            expires_at,
            rental_fee_paid: listing.rental_fee,
            collateral_escrowed: listing.collateral_amount,
            is_active: true,
        };

        env.storage().instance().set(&DataKey::Agreement(token_id), &agreement);
        env.events().publish(
            (symbol_short!("rent"), token_id),
            (renter, expires_at),
        );
    }

    /// Terminate rental early by renter. Revokes usability rights and refunds collateral immediately.
    pub fn terminate_early(env: Env, token_id: u64, renter: Address) {
        renter.require_auth();

        let mut agreement = Self::get_rental(env.clone(), token_id);
        if !agreement.is_active {
            panic!("rental agreement is not active");
        }
        if agreement.renter != renter {
            panic!("caller is not the active renter");
        }

        agreement.is_active = false;
        env.storage().instance().set(&DataKey::Agreement(token_id), &agreement);

        // Refund collateral to renter
        if agreement.collateral_escrowed > 0 {
            token::TokenClient::new(&env, &agreement.payment_token).transfer(
                &env.current_contract_address(),
                &renter,
                &agreement.collateral_escrowed,
            );
        }

        env.events().publish(
            (symbol_short!("termevnt"), token_id),
            renter,
        );
    }

    /// Reclaim expired rental and refund escrowed collateral back to renter.
    pub fn reclaim_expired(env: Env, token_id: u64) {
        let mut agreement = Self::get_rental(env.clone(), token_id);
        if !agreement.is_active {
            panic!("rental is not active");
        }

        let now = env.ledger().timestamp();
        if now < agreement.expires_at {
            panic!("rental has not expired yet");
        }

        agreement.is_active = false;
        env.storage().instance().set(&DataKey::Agreement(token_id), &agreement);

        if agreement.collateral_escrowed > 0 {
            token::TokenClient::new(&env, &agreement.payment_token).transfer(
                &env.current_contract_address(),
                &agreement.renter,
                &agreement.collateral_escrowed,
            );
        }

        env.events().publish(
            (symbol_short!("expired"), token_id),
            agreement.renter,
        );
    }

    /// Return the immutable owner of the NFT.
    pub fn get_owner(env: Env, token_id: u64) -> Address {
        let nft: NFT = env
            .storage()
            .instance()
            .get(&DataKey::NFT(token_id))
            .unwrap_or_else(|| panic!("nft not found"));
        nft.owner
    }

    /// Return the current authorized user (renter) if active and unexpired, otherwise None.
    pub fn get_user(env: Env, token_id: u64) -> Option<Address> {
        let maybe_agreement: core::option::Option<RentalAgreement> =
            env.storage().instance().get(&DataKey::Agreement(token_id));

        if let core::option::Option::Some(agreement) = maybe_agreement {
            let now = env.ledger().timestamp();
            if agreement.is_active && now < agreement.expires_at {
                return Option::Some(agreement.renter);
            }
        }
        Option::None
    }

    /// Check if the NFT is currently under an active, unexpired rental.
    pub fn is_rented(env: Env, token_id: u64) -> bool {
        let maybe_agreement: core::option::Option<RentalAgreement> =
            env.storage().instance().get(&DataKey::Agreement(token_id));

        if let core::option::Option::Some(agreement) = maybe_agreement {
            let now = env.ledger().timestamp();
            agreement.is_active && now < agreement.expires_at
        } else {
            false
        }
    }

    /// Return NFT metadata record.
    pub fn get_nft(env: Env, token_id: u64) -> NFT {
        env.storage()
            .instance()
            .get(&DataKey::NFT(token_id))
            .unwrap_or_else(|| panic!("nft not found"))
    }

    /// Return listing details.
    pub fn get_listing(env: Env, token_id: u64) -> RentalListing {
        env.storage()
            .instance()
            .get(&DataKey::Listing(token_id))
            .unwrap_or_else(|| panic!("listing not found"))
    }

    /// Return rental agreement details.
    pub fn get_rental(env: Env, token_id: u64) -> RentalAgreement {
        env.storage()
            .instance()
            .get(&DataKey::Agreement(token_id))
            .unwrap_or_else(|| panic!("rental agreement not found"))
    }
}

#[cfg(test)]
mod test;
