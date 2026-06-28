#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Vec};

#[contracttype]
#[derive(Clone)]
struct PriceData {
    symbol: String,
    price: i128,
    timestamp: u64,
    source: String,
}

#[contracttype]
#[derive(Clone)]
struct DataSource {
    source_id: u64,
    address: Address,
    name: String,
    active: bool,
    last_update: u64,
}

#[contracttype]
enum DataKey {
    Admin,
    SourceCounter,
    DataSource(u64),
    Price(String),
    PriceHistory(String, u64),
    SourceWhitelist(Address),
}

/// Oracle Contract with multiple data sources
#[contract]
#[derive(Default)]
pub struct Oracle;

#[contractimpl]
impl Oracle {
    /// Initialize oracle contract
    pub fn initialize(env: Env, admin: Address) {
        let storage = env.storage().instance();
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::SourceCounter, &0u64);
    }

    /// Register a new data source
    pub fn register_source(env: Env, address: Address, name: String) -> Result<u64, &'static str> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();
        let mut counter: u64 = storage.get(&DataKey::SourceCounter).unwrap_or(0);
        counter += 1;

        let source = DataSource {
            source_id: counter,
            address: address.clone(),
            name,
            active: true,
            last_update: env.ledger().timestamp(),
        };

        storage.set(&DataKey::DataSource(counter), &source);
        storage.set(&DataKey::SourceCounter, &counter);
        storage.set(&DataKey::SourceWhitelist(address), &true);

        env.events()
            .publish((symbol_short!("source"), counter), 1);

        Ok(counter)
    }

    /// Submit price data from a source
    pub fn submit_price(
        env: Env,
        symbol: String,
        price: i128,
        source_name: String,
    ) -> Result<(), &'static str> {
        let sender = env.current_contract_address();

        let storage = env.storage().instance();

        // Verify sender is whitelisted
        let is_whitelisted: bool = storage
            .get(&DataKey::SourceWhitelist(sender.clone()))
            .unwrap_or(false);

        if !is_whitelisted {
            return Err("Source not whitelisted");
        }

        if price <= 0 {
            return Err("Price must be positive");
        }

        let timestamp = env.ledger().timestamp();

        // Store latest price
        let price_data = PriceData {
            symbol: symbol.clone(),
            price,
            timestamp,
            source: source_name,
        };

        storage.set(&DataKey::Price(symbol.clone()), &price_data);

        // Store in history
        storage.set(
            &DataKey::PriceHistory(symbol.clone(), timestamp),
            &price,
        );

        env.events()
            .publish((symbol_short!("price"), symbol), price);

        Ok(())
    }

    /// Get latest price for a symbol
    pub fn get_price(env: Env, symbol: String) -> Result<i128, &'static str> {
        env.storage()
            .instance()
            .get::<_, PriceData>(&DataKey::Price(symbol))
            .map(|data| data.price)
            .ok_or("Price not found")
    }

    /// Get price data with source info
    pub fn get_price_data(env: Env, symbol: String) -> Result<PriceData, &'static str> {
        env.storage()
            .instance()
            .get(&DataKey::Price(symbol))
            .ok_or("Price not found")
    }

    /// Aggregate prices from multiple sources (average)
    pub fn aggregate_price(env: Env, symbol: String, num_sources: u64) -> Result<i128, &'static str> {
        if num_sources == 0 {
            return Err("num_sources must be positive");
        }

        let storage = env.storage().instance();

        // For MVP, return the latest price
        // In production, this would aggregate from multiple sources
        let price_data: PriceData = storage
            .get(&DataKey::Price(symbol))
            .ok_or("Price not found")?;

        Ok(price_data.price)
    }

    /// Get data source details
    pub fn get_source(env: Env, source_id: u64) -> Result<DataSource, &'static str> {
        env.storage()
            .instance()
            .get(&DataKey::DataSource(source_id))
            .ok_or("Source not found")
    }

    /// Deactivate a data source
    pub fn deactivate_source(env: Env, source_id: u64) -> Result<(), &'static str> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();

        let mut source: DataSource = storage
            .get(&DataKey::DataSource(source_id))
            .ok_or("Source not found")?;

        source.active = false;
        storage.set(&DataKey::DataSource(source_id), &source);

        // Remove from whitelist
        storage.set(&DataKey::SourceWhitelist(source.address), &false);

        env.events()
            .publish((symbol_short!("deact"), source_id), 0);

        Ok(())
    }

    /// Activate a data source
    pub fn activate_source(env: Env, source_id: u64) -> Result<(), &'static str> {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let storage = env.storage().instance();

        let mut source: DataSource = storage
            .get(&DataKey::DataSource(source_id))
            .ok_or("Source not found")?;

        source.active = true;
        source.last_update = env.ledger().timestamp();
        storage.set(&DataKey::DataSource(source_id), &source);

        // Add to whitelist
        storage.set(&DataKey::SourceWhitelist(source.address), &true);

        env.events()
            .publish((symbol_short!("actv"), source_id), 1);

        Ok(())
    }

    /// Validate price data freshness
    pub fn validate_price_freshness(
        env: Env,
        symbol: String,
        max_age_seconds: u64,
    ) -> Result<bool, &'static str> {
        let storage = env.storage().instance();

        let price_data: PriceData = storage
            .get(&DataKey::Price(symbol))
            .ok_or("Price not found")?;

        let age = env.ledger().timestamp() - price_data.timestamp;
        Ok(age <= max_age_seconds)
    }
}
