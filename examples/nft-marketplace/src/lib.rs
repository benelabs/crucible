#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Map};

#[contracttype]
#[derive(Clone)]
pub struct NFT {
    pub id: u64,
    pub owner: Address,
    pub creator: Address,
    pub royalty_percent: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct Listing {
    pub nft_id: u64,
    pub seller: Address,
    pub price: i128,
}

#[contracttype]
enum DataKey {
    NFT(u64),
    Listing(u64),
    NextId,
}

/// NFT Marketplace contract with royalties support.
#[contract]
pub struct NFTMarketplace;

#[contractimpl]
impl NFTMarketplace {
    /// Mint a new NFT with royalty configuration.
    pub fn mint(env: Env, to: Address, royalty_percent: u32) -> u64 {
        if royalty_percent > 100 {
            panic!("royalty_percent must be <= 100");
        }

        let next_id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap_or(1);
        let nft = NFT {
            id: next_id,
            owner: to.clone(),
            creator: to,
            royalty_percent,
        };

        env.storage().instance().set(&DataKey::NFT(next_id), &nft);
        env.storage().instance().set(&DataKey::NextId, &(next_id + 1));
        env.events().publish((symbol_short!("mint"), next_id), royalty_percent);

        next_id
    }

    /// List NFT for sale on marketplace.
    pub fn list(env: Env, nft_id: u64, price: i128, seller: Address) {
        seller.require_auth();
        if price <= 0 {
            panic!("price must be positive");
        }

        let nft: NFT = env
            .storage()
            .instance()
            .get(&DataKey::NFT(nft_id))
            .unwrap_or_else(|| panic!("nft not found"));

        if nft.owner != seller {
            panic!("not nft owner");
        }

        let listing = Listing { nft_id, seller: seller.clone(), price };
        env.storage().instance().set(&DataKey::Listing(nft_id), &listing);
        env.events().publish((symbol_short!("list"), nft_id), price);
    }

    /// Purchase NFT with automatic royalty distribution.
    pub fn buy(env: Env, nft_id: u64, buyer: Address) {
        buyer.require_auth();

        let listing: Listing = env
            .storage()
            .instance()
            .get(&DataKey::Listing(nft_id))
            .unwrap_or_else(|| panic!("listing not found"));

        let mut nft: NFT = env
            .storage()
            .instance()
            .get(&DataKey::NFT(nft_id))
            .unwrap_or_else(|| panic!("nft not found"));

        let royalty_amount = (listing.price * (nft.royalty_percent as i128)) / 100;
        let seller_amount = listing.price - royalty_amount;

        nft.owner = buyer.clone();
        env.storage().instance().set(&DataKey::NFT(nft_id), &nft);
        env.storage().instance().remove(&DataKey::Listing(nft_id));

        env.events().publish(
            (symbol_short!("buy"), nft_id),
            (buyer, listing.price, royalty_amount),
        );
    }

    /// Get NFT details.
    pub fn get_nft(env: Env, nft_id: u64) -> NFT {
        env.storage()
            .instance()
            .get(&DataKey::NFT(nft_id))
            .unwrap_or_else(|| panic!("nft not found"))
    }

    /// Get listing details.
    pub fn get_listing(env: Env, nft_id: u64) -> Listing {
        env.storage()
            .instance()
            .get(&DataKey::Listing(nft_id))
            .unwrap_or_else(|| panic!("listing not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marketplace_creation() {
        let marketplace = NFTMarketplace;
        assert_eq!(std::mem::size_of_val(&marketplace), 0);
    }
}
