#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, String, Vec};

/// Maximum age of a price observation before it is considered stale (1 hour).
const MAX_STALENESS_SECONDS: u64 = 3_600;

/// Maximum allowed price deviation per update: 15%
const MAX_PRICE_DEVIATION_BPS: i128 = 1500; // basis points (15% = 1500 bps)

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
        source: Address,
        symbol: String,
        price: i128,
        source_name: String,
    ) -> Result<(), &'static str> {
        source.require_auth();

        let storage = env.storage().instance();

        // Verify source address is whitelisted
        let is_whitelisted: bool = storage
            .get(&DataKey::SourceWhitelist(source.clone()))
            .unwrap_or(false);

        if !is_whitelisted {
            return Err("Source not whitelisted");
        }

        if price <= 0 {
            return Err("Price must be positive");
        }

        // Check price deviation against previous price
        if let Some(prev_price_data) = storage.get::<_, PriceData>(&DataKey::Price(symbol.clone())) {
            let prev_price = prev_price_data.price;
            if prev_price > 0 {
                // Calculate absolute percentage deviation in basis points
                let price_diff = (price - prev_price).abs();
                let deviation_bps = (price_diff * 10000) / prev_price;

                if deviation_bps > MAX_PRICE_DEVIATION_BPS {
                    return Err("Price deviation exceeds maximum threshold of 15%");
                }
            }
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

    /// Get latest price for a symbol.
    /// Reverts if the observation is older than [`MAX_STALENESS_SECONDS`].
    pub fn get_price(env: Env, symbol: String) -> Result<i128, &'static str> {
        let data = Self::load_fresh_price(&env, symbol)?;
        Ok(data.price)
    }

    /// Get price data with source info.
    /// Reverts if the observation is older than [`MAX_STALENESS_SECONDS`].
    pub fn get_price_data(env: Env, symbol: String) -> Result<PriceData, &'static str> {
        Self::load_fresh_price(&env, symbol)
    }

    /// Aggregate prices from multiple sources using median
    pub fn aggregate_price(env: Env, symbol: String, num_sources: u64) -> Result<i128, &'static str> {
        if num_sources == 0 {
            return Err("num_sources must be positive");
        }

        // For MVP, return the latest fresh price.
        // In production, this would aggregate from multiple sources.
        let price_data = Self::load_fresh_price(&env, symbol)?;
        Ok(price_data.price)
    }

        // Get current price and verify we have enough data
        let current_price: PriceData = storage
            .get(&DataKey::Price(symbol.clone()))
            .ok_or("Price not found")?;

        // Count active sources
        let source_counter: u64 = storage.get(&DataKey::SourceCounter).unwrap_or(0);
        let mut active_count = 0u64;

        for source_id in 1..=source_counter {
            if let Some(source) = storage.get::<_, DataSource>(&DataKey::DataSource(source_id)) {
                if source.active {
                    active_count += 1;
                }
            }
        }

        // If num_sources is 1 or we have fewer active sources, return current price
        if num_sources == 1 || active_count < 2 {
            return Ok(current_price.price);
        }

        // For median calculation, we use the current price as one data point
        // and simulate getting prices from multiple sources
        // In production, each source would have its own price entry
        let mut prices: Vec<i128> = Vec::new(&env);
        
        // Add current price as the base
        prices.push_back(current_price.price);

        // For now, we use the price history to gather additional price points
        // This is a simplified implementation - production would track per-source prices
        let current_time = env.ledger().timestamp();
        
        // Try to get historical prices from the last hour (3600 seconds)
        let lookback = 3600u64;
        let start_time = current_time.saturating_sub(lookback);
        
        // Collect up to num_sources price points from history
        let mut found = 1u64;
        let mut t = start_time;
        
        while found < num_sources && t < current_time {
            if let Some(price) = storage.get::<_, i128>(&DataKey::PriceHistory(symbol.clone(), t)) {
                if price != current_price.price {
                    prices.push_back(price);
                    found += 1;
                    if found >= num_sources {
                        break;
                    }
                }
            }
            t += 1; // Check each second
        }

        if prices.len() < 2 {
            // Not enough historical data, return current price with warning
            return Ok(current_price.price);
        }

        // Sort prices to find median
        let len = prices.len();
        
        // Simple selection sort for small arrays
        for i in 0..len {
            let mut min_idx = i;
            for j in (i + 1)..len {
                if prices.get(j).unwrap_or(&i128::MAX) < prices.get(min_idx).unwrap_or(&i128::MAX) {
                    min_idx = j;
                }
            }
            if min_idx != i {
                let temp = *prices.get(i).unwrap_or(&0);
                let min_val = *prices.get(min_idx).unwrap_or(&0);
                // Manual swap since we can't do tuple assignment easily
                prices.set(i, min_val);
                prices.set(min_idx, temp);
            }
        }

        // Return median
        let mid = prices.len() / 2;
        if prices.len() % 2 == 0 {
            // Even: average of two middle values
            let v1 = *prices.get(mid - 1).unwrap_or(&0);
            let v2 = *prices.get(mid).unwrap_or(&0);
            Ok((v1 + v2) / 2)
        } else {
            // Odd: middle value
            Ok(*prices.get(mid).unwrap_or(&0))
        }
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

    /// Validate price data freshness against a caller-supplied max age.
    pub fn validate_price_freshness(
        env: Env,
        symbol: String,
        max_age_seconds: u64,
    ) -> Result<bool, &'static str> {
        let storage = env.storage().instance();

        let price_data: PriceData = storage
            .get(&DataKey::Price(symbol))
            .ok_or("Price not found")?;

        // Use saturating subtraction to prevent underflow if clock drifts backward
        let current_time = env.ledger().timestamp();
        let age = current_time.saturating_sub(price_data.timestamp);
        Ok(age <= max_age_seconds)
    }
}

#[cfg(test)]
mod test;

