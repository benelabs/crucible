#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum StreamStatus {
    Active,
    Cancelled,
    Completed,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Stream {
    pub sender: Address,
    pub recipient: Address,
    pub token: Address,
    pub deposit_amount: i128,
    pub claimed_amount: i128,
    pub start_time: u64,
    pub stop_time: u64,
    pub status: StreamStatus,
}

#[contracttype]
pub enum DataKey {
    Stream(u64),
    NextStreamId,
}

#[contract]
#[derive(Default)]
pub struct PaymentStreaming;

#[contractimpl]
impl PaymentStreaming {
    /// Create a new continuous payment stream.
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit_amount: i128,
        start_time: u64,
        stop_time: u64,
    ) -> u64 {
        sender.require_auth();
        assert!(deposit_amount > 0, "Deposit amount must be positive");
        assert!(start_time < stop_time, "Start time must be before stop time");

        let current_time = env.ledger().timestamp();
        assert!(stop_time > current_time, "Stop time must be in future");

        // Transfer tokens from sender to contract
        let client = token::Client::new(&env, &token);
        client.transfer(&sender, &env.current_contract_address(), &deposit_amount);

        let stream_id: u64 = env.storage().instance().get(&DataKey::NextStreamId).unwrap_or(1);
        env.storage().instance().set(&DataKey::NextStreamId, &(stream_id + 1));

        let stream = Stream {
            sender: sender.clone(),
            recipient: recipient.clone(),
            token: token.clone(),
            deposit_amount,
            claimed_amount: 0,
            start_time,
            stop_time,
            status: StreamStatus::Active,
        };

        env.storage().persistent().set(&DataKey::Stream(stream_id), &stream);

        env.events().publish(
            (symbol_short!("created"), sender, recipient),
            (stream_id, deposit_amount),
        );

        stream_id
    }

    /// Calculate claimable amount based on elapsed time: (deposit * elapsed) / duration - claimed.
    pub fn claimable_amount(env: Env, stream_id: u64) -> i128 {
        let stream: Stream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .expect("Stream not found");

        if stream.status != StreamStatus::Active {
            return 0;
        }

        let now = env.ledger().timestamp();
        if now <= stream.start_time {
            return 0;
        }

        let duration = (stream.stop_time - stream.start_time) as i128;
        let elapsed = if now >= stream.stop_time {
            duration
        } else {
            (now - stream.start_time) as i128
        };

        let vested = (stream.deposit_amount * elapsed) / duration;
        vested - stream.claimed_amount
    }

    /// Withdraw claimable streamed funds for the recipient.
    pub fn withdraw_from_stream(env: Env, recipient: Address, stream_id: u64) -> i128 {
        recipient.require_auth();

        let mut stream: Stream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .expect("Stream not found");

        assert_eq!(stream.recipient, recipient, "Caller is not stream recipient");
        assert_eq!(stream.status, StreamStatus::Active, "Stream is not active");

        let amount = Self::claimable_amount(env.clone(), stream_id);
        assert!(amount > 0, "No claimable funds available");

        stream.claimed_amount += amount;
        if stream.claimed_amount >= stream.deposit_amount {
            stream.status = StreamStatus::Completed;
        }

        env.storage().persistent().set(&DataKey::Stream(stream_id), &stream);

        let client = token::Client::new(&env, &stream.token);
        client.transfer(&env.current_contract_address(), &recipient, &amount);

        env.events().publish(
            (symbol_short!("withdraw"), recipient),
            (stream_id, amount),
        );

        amount
    }

    /// Cancel stream with proportional refund to sender and claimable to recipient.
    pub fn cancel_stream(env: Env, sender: Address, stream_id: u64) {
        sender.require_auth();

        let mut stream: Stream = env
            .storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .expect("Stream not found");

        assert_eq!(stream.sender, sender, "Caller is not stream sender");
        assert_eq!(stream.status, StreamStatus::Active, "Stream is not active");

        let claimable = Self::claimable_amount(env.clone(), stream_id);
        let refund = stream.deposit_amount - stream.claimed_amount - claimable;

        stream.status = StreamStatus::Cancelled;
        env.storage().persistent().set(&DataKey::Stream(stream_id), &stream);

        let client = token::Client::new(&env, &stream.token);
        if claimable > 0 {
            client.transfer(&env.current_contract_address(), &stream.recipient, &claimable);
        }
        if refund > 0 {
            client.transfer(&env.current_contract_address(), &sender, &refund);
        }

        env.events().publish(
            (symbol_short!("cancelled"), sender),
            (stream_id, refund),
        );
    }
}

#[cfg(test)]
mod test;
