#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlStateQuery {
    pub field_name: String,
    pub include_events: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlashLoanPremiumCurve {
    pub base_bps: u32,
    pub utilization_bps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtlcSwapWindow {
    pub hash_algorithm: String,
    pub timelock_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityAttestation {
    pub subject: String,
    pub issuer: String,
    pub revoked: bool,
}

pub fn default_state_query() -> GraphqlStateQuery {
    GraphqlStateQuery {
        field_name: "contractState".to_string(),
        include_events: true,
    }
}

pub fn premium_bps(curve: &FlashLoanPremiumCurve) -> u32 {
    curve.base_bps.saturating_add(curve.utilization_bps / 10)
}

pub fn active_attestation_count(attestations: &[IdentityAttestation]) -> usize {
    attestations.iter().filter(|item| !item.revoked).count()
}
