#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelockVote {
    pub proposal_id: u64,
    pub approvals: u32,
    pub voting_window_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderbookLevel {
    pub price: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CdpPosition {
    pub collateral_value: f64,
    pub debt_value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoyaltyRecipient {
    pub account: String,
    pub bps: u32,
}

pub fn has_quorum(vote: &TimelockVote, threshold: u32) -> bool {
    vote.approvals >= threshold
}

pub fn cdp_health_factor(position: &CdpPosition) -> f64 {
    if position.debt_value <= 0.0 {
        return f64::INFINITY;
    }

    position.collateral_value / position.debt_value
}

pub fn total_royalty_bps(recipients: &[RoyaltyRecipient]) -> u32 {
    recipients.iter().map(|item| item.bps).sum()
}
