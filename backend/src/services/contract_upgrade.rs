use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

const REQUIRED_CHECKS: [&str; 3] = ["storage_layout", "public_interface", "authorization"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContractUpgradeRequest {
    pub contract_id: String,
    pub current_version: String,
    pub target_version: String,
    pub current_wasm_hash: String,
    pub target_wasm_hash: String,
    pub requested_by: String,
    pub strategy: UpgradeStrategy,
    #[serde(default)]
    pub migration_required: bool,
    pub state_migration_hash: Option<String>,
    #[serde(default)]
    pub compatibility_checks: Vec<CompatibilityCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityCheck {
    pub name: String,
    pub passed: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpgradeStrategy {
    InPlace,
    StateMigration,
    RedeployAndMigrate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpgradePlanStatus {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpgradeRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpgradeAction {
    FreezeWrites,
    SnapshotState,
    VerifyCompatibility,
    UploadWasm,
    RunStateMigration,
    SwitchContract,
    VerifyPostUpgrade,
    UnfreezeWrites,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeStep {
    pub order: u8,
    pub action: UpgradeAction,
    pub description: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RollbackPlan {
    pub available: bool,
    pub restore_wasm_hash: String,
    pub restore_version: String,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecurityReviewSummary {
    pub required: bool,
    pub blocking_findings: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContractUpgradePlan {
    pub plan_id: String,
    pub contract_id: String,
    pub from_version: String,
    pub to_version: String,
    pub from_wasm_hash: String,
    pub to_wasm_hash: String,
    pub requested_by: String,
    pub strategy: UpgradeStrategy,
    pub status: UpgradePlanStatus,
    pub risk_level: UpgradeRiskLevel,
    pub approvals_required: u8,
    pub blockers: Vec<String>,
    pub steps: Vec<UpgradeStep>,
    pub rollback: RollbackPlan,
    pub security_review: SecurityReviewSummary,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractUpgradeError {
    EmptyField(&'static str),
    InvalidVersion(String),
    NonIncreasingVersion,
    IdenticalWasmHash,
    MissingMigrationHash,
    InvalidStrategy(String),
}

impl fmt::Display for ContractUpgradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "{field} is required"),
            Self::InvalidVersion(version) => write!(f, "invalid semantic version: {version}"),
            Self::NonIncreasingVersion => {
                write!(f, "target version must be greater than current version")
            }
            Self::IdenticalWasmHash => write!(f, "target wasm hash must differ from current hash"),
            Self::MissingMigrationHash => {
                write!(f, "state migration hash is required for this strategy")
            }
            Self::InvalidStrategy(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ContractUpgradeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    fn parse(input: &str) -> Result<Self, ContractUpgradeError> {
        let normalized = input.strip_prefix('v').unwrap_or(input);
        let mut parts = normalized.split('.');
        let major = Self::parse_part(parts.next(), input)?;
        let minor = Self::parse_part(parts.next(), input)?;
        let patch = Self::parse_part(parts.next(), input)?;
        if parts.next().is_some() {
            return Err(ContractUpgradeError::InvalidVersion(input.to_string()));
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    fn parse_part(part: Option<&str>, original: &str) -> Result<u64, ContractUpgradeError> {
        let part =
            part.ok_or_else(|| ContractUpgradeError::InvalidVersion(original.to_string()))?;
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(ContractUpgradeError::InvalidVersion(original.to_string()));
        }
        part.parse()
            .map_err(|_| ContractUpgradeError::InvalidVersion(original.to_string()))
    }

    fn is_greater_than(self, other: Self) -> bool {
        (self.major, self.minor, self.patch) > (other.major, other.minor, other.patch)
    }

    fn is_major_upgrade_from(self, other: Self) -> bool {
        self.major > other.major
    }

    fn is_patch_upgrade_from(self, other: Self) -> bool {
        self.major == other.major && self.minor == other.minor && self.patch > other.patch
    }
}

#[derive(Debug, Default, Clone)]
pub struct ContractUpgradeManager;

impl ContractUpgradeManager {
    pub fn new() -> Self {
        Self
    }

    pub fn plan_upgrade(
        &self,
        request: ContractUpgradeRequest,
    ) -> Result<ContractUpgradePlan, ContractUpgradeError> {
        validate_required_fields(&request)?;
        validate_strategy(&request)?;

        let current = Version::parse(&request.current_version)?;
        let target = Version::parse(&request.target_version)?;
        if !target.is_greater_than(current) {
            return Err(ContractUpgradeError::NonIncreasingVersion);
        }
        if request.current_wasm_hash == request.target_wasm_hash {
            return Err(ContractUpgradeError::IdenticalWasmHash);
        }

        let blockers = compatibility_blockers(&request);
        let status = if blockers.is_empty() {
            UpgradePlanStatus::Ready
        } else {
            UpgradePlanStatus::Blocked
        };
        let risk_level = risk_level(&request, current, target, &blockers);
        let approvals_required = approvals_required(&risk_level, &status);

        let security_review = SecurityReviewSummary {
            required: true,
            blocking_findings: blockers.clone(),
            notes: security_notes(&request, current, target),
        };

        Ok(ContractUpgradePlan {
            plan_id: plan_id(&request),
            contract_id: request.contract_id.clone(),
            from_version: request.current_version.clone(),
            to_version: request.target_version.clone(),
            from_wasm_hash: request.current_wasm_hash.clone(),
            to_wasm_hash: request.target_wasm_hash.clone(),
            requested_by: request.requested_by.clone(),
            strategy: request.strategy.clone(),
            status,
            risk_level,
            approvals_required,
            blockers: blockers.clone(),
            steps: upgrade_steps(&request),
            rollback: RollbackPlan {
                available: true,
                restore_wasm_hash: request.current_wasm_hash,
                restore_version: request.current_version,
                steps: vec![
                    "freeze contract writes".to_string(),
                    "restore previous wasm hash".to_string(),
                    "replay pre-upgrade state snapshot if migration was applied".to_string(),
                    "run post-rollback health checks".to_string(),
                ],
            },
            security_review,
            created_at: Utc::now(),
        })
    }
}

fn validate_required_fields(request: &ContractUpgradeRequest) -> Result<(), ContractUpgradeError> {
    for (field, value) in [
        ("contract_id", request.contract_id.as_str()),
        ("current_version", request.current_version.as_str()),
        ("target_version", request.target_version.as_str()),
        ("current_wasm_hash", request.current_wasm_hash.as_str()),
        ("target_wasm_hash", request.target_wasm_hash.as_str()),
        ("requested_by", request.requested_by.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ContractUpgradeError::EmptyField(field));
        }
    }
    Ok(())
}

fn validate_strategy(request: &ContractUpgradeRequest) -> Result<(), ContractUpgradeError> {
    let migration_strategy = matches!(
        request.strategy,
        UpgradeStrategy::StateMigration | UpgradeStrategy::RedeployAndMigrate
    );

    if request.migration_required && !migration_strategy {
        return Err(ContractUpgradeError::InvalidStrategy(
            "migration_required cannot use in-place strategy".to_string(),
        ));
    }

    if migration_strategy
        && request
            .state_migration_hash
            .as_ref()
            .map(|hash| hash.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(ContractUpgradeError::MissingMigrationHash);
    }

    Ok(())
}

fn compatibility_blockers(request: &ContractUpgradeRequest) -> Vec<String> {
    let mut blockers = Vec::new();

    for required in REQUIRED_CHECKS {
        match request
            .compatibility_checks
            .iter()
            .find(|check| check.name == required)
        {
            Some(check) if check.passed => {}
            Some(check) => blockers.push(format!(
                "compatibility check failed: {}{}",
                check.name,
                check
                    .notes
                    .as_ref()
                    .map(|notes| format!(" ({notes})"))
                    .unwrap_or_default()
            )),
            None => blockers.push(format!("compatibility check missing: {required}")),
        }
    }

    blockers
}

fn risk_level(
    request: &ContractUpgradeRequest,
    current: Version,
    target: Version,
    blockers: &[String],
) -> UpgradeRiskLevel {
    if !blockers.is_empty()
        || target.is_major_upgrade_from(current)
        || matches!(request.strategy, UpgradeStrategy::RedeployAndMigrate)
    {
        return UpgradeRiskLevel::High;
    }

    if request.migration_required
        || matches!(request.strategy, UpgradeStrategy::StateMigration)
        || !target.is_patch_upgrade_from(current)
    {
        return UpgradeRiskLevel::Medium;
    }

    UpgradeRiskLevel::Low
}

fn approvals_required(risk_level: &UpgradeRiskLevel, status: &UpgradePlanStatus) -> u8 {
    if *status == UpgradePlanStatus::Blocked {
        return 0;
    }

    match risk_level {
        UpgradeRiskLevel::Low => 1,
        UpgradeRiskLevel::Medium => 2,
        UpgradeRiskLevel::High => 3,
    }
}

fn upgrade_steps(request: &ContractUpgradeRequest) -> Vec<UpgradeStep> {
    let mut steps = vec![
        step(1, UpgradeAction::FreezeWrites, "freeze contract writes"),
        step(
            2,
            UpgradeAction::SnapshotState,
            "snapshot current contract state",
        ),
        step(
            3,
            UpgradeAction::VerifyCompatibility,
            "verify storage, interface, and authorization compatibility",
        ),
        step(4, UpgradeAction::UploadWasm, "upload target contract wasm"),
    ];

    if matches!(
        request.strategy,
        UpgradeStrategy::StateMigration | UpgradeStrategy::RedeployAndMigrate
    ) {
        steps.push(step(
            5,
            UpgradeAction::RunStateMigration,
            "run audited state migration artifact",
        ));
    }

    steps.push(step(
        next_order(&steps),
        UpgradeAction::SwitchContract,
        "activate target implementation",
    ));
    steps.push(step(
        next_order(&steps),
        UpgradeAction::VerifyPostUpgrade,
        "run post-upgrade health and invariant checks",
    ));
    steps.push(step(
        next_order(&steps),
        UpgradeAction::UnfreezeWrites,
        "unfreeze contract writes",
    ));

    steps
}

fn step(order: u8, action: UpgradeAction, description: &str) -> UpgradeStep {
    UpgradeStep {
        order,
        action,
        description: description.to_string(),
        required: true,
    }
}

fn next_order(steps: &[UpgradeStep]) -> u8 {
    steps.len() as u8 + 1
}

fn security_notes(
    request: &ContractUpgradeRequest,
    current: Version,
    target: Version,
) -> Vec<String> {
    let mut notes = vec![
        "verify target wasm hash against build provenance before execution".to_string(),
        "confirm rollback artifact remains deployable until upgrade is finalized".to_string(),
    ];

    if target.is_major_upgrade_from(current) {
        notes.push("major version upgrade requires expanded reviewer sign-off".to_string());
    }
    if request.migration_required {
        notes.push("state migration must be reviewed and dry-run before activation".to_string());
    }

    notes
}

fn plan_id(request: &ContractUpgradeRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.contract_id.as_bytes());
    hasher.update(request.current_version.as_bytes());
    hasher.update(request.target_version.as_bytes());
    hasher.update(request.current_wasm_hash.as_bytes());
    hasher.update(request.target_wasm_hash.as_bytes());
    let digest = hasher.finalize();
    let hex_digest = digest.iter().map(|b| format!("{:02x}", b)).collect::<String>();
    format!("upg-{}", hex_digest)[..20].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_check(name: &str) -> CompatibilityCheck {
        CompatibilityCheck {
            name: name.to_string(),
            passed: true,
            notes: None,
        }
    }

    fn base_request() -> ContractUpgradeRequest {
        ContractUpgradeRequest {
            contract_id: "CCONTRACT123".to_string(),
            current_version: "1.2.3".to_string(),
            target_version: "1.2.4".to_string(),
            current_wasm_hash: "wasm-old".to_string(),
            target_wasm_hash: "wasm-new".to_string(),
            requested_by: "GADMIN123".to_string(),
            strategy: UpgradeStrategy::InPlace,
            migration_required: false,
            state_migration_hash: None,
            compatibility_checks: REQUIRED_CHECKS
                .iter()
                .map(|name| passing_check(name))
                .collect(),
        }
    }

    #[test]
    fn creates_ready_low_risk_patch_plan() {
        let plan = ContractUpgradeManager::new()
            .plan_upgrade(base_request())
            .unwrap();

        assert_eq!(plan.status, UpgradePlanStatus::Ready);
        assert_eq!(plan.risk_level, UpgradeRiskLevel::Low);
        assert_eq!(plan.approvals_required, 1);
        assert_eq!(plan.steps.len(), 7);
        assert_eq!(plan.rollback.restore_version, "1.2.3");
    }

    #[test]
    fn migration_strategy_adds_migration_step_and_medium_risk() {
        let mut request = base_request();
        request.target_version = "1.3.0".to_string();
        request.strategy = UpgradeStrategy::StateMigration;
        request.migration_required = true;
        request.state_migration_hash = Some("migration-wasm".to_string());

        let plan = ContractUpgradeManager::new().plan_upgrade(request).unwrap();

        assert_eq!(plan.risk_level, UpgradeRiskLevel::Medium);
        assert_eq!(plan.approvals_required, 2);
        assert!(plan
            .steps
            .iter()
            .any(|step| step.action == UpgradeAction::RunStateMigration));
    }

    #[test]
    fn major_upgrade_is_high_risk() {
        let mut request = base_request();
        request.target_version = "2.0.0".to_string();

        let plan = ContractUpgradeManager::new().plan_upgrade(request).unwrap();

        assert_eq!(plan.risk_level, UpgradeRiskLevel::High);
        assert_eq!(plan.approvals_required, 3);
    }

    #[test]
    fn failed_compatibility_check_blocks_plan() {
        let mut request = base_request();
        request.compatibility_checks[0].passed = false;
        request.compatibility_checks[0].notes = Some("layout slot changed".to_string());

        let plan = ContractUpgradeManager::new().plan_upgrade(request).unwrap();

        assert_eq!(plan.status, UpgradePlanStatus::Blocked);
        assert_eq!(plan.approvals_required, 0);
        assert!(plan.blockers[0].contains("storage_layout"));
    }

    #[test]
    fn missing_required_check_blocks_plan() {
        let mut request = base_request();
        request
            .compatibility_checks
            .retain(|check| check.name != "authorization");

        let plan = ContractUpgradeManager::new().plan_upgrade(request).unwrap();

        assert_eq!(plan.status, UpgradePlanStatus::Blocked);
        assert!(plan
            .blockers
            .iter()
            .any(|blocker| blocker.contains("authorization")));
    }

    #[test]
    fn rejects_non_increasing_versions() {
        let mut request = base_request();
        request.target_version = "1.2.3".to_string();

        let err = ContractUpgradeManager::new()
            .plan_upgrade(request)
            .unwrap_err();

        assert_eq!(err, ContractUpgradeError::NonIncreasingVersion);
    }

    #[test]
    fn rejects_identical_wasm_hashes() {
        let mut request = base_request();
        request.target_wasm_hash = request.current_wasm_hash.clone();

        let err = ContractUpgradeManager::new()
            .plan_upgrade(request)
            .unwrap_err();

        assert_eq!(err, ContractUpgradeError::IdenticalWasmHash);
    }

    #[test]
    fn rejects_migration_without_migration_hash() {
        let mut request = base_request();
        request.strategy = UpgradeStrategy::StateMigration;

        let err = ContractUpgradeManager::new()
            .plan_upgrade(request)
            .unwrap_err();

        assert_eq!(err, ContractUpgradeError::MissingMigrationHash);
    }

    #[test]
    fn rejects_in_place_strategy_when_migration_is_required() {
        let mut request = base_request();
        request.migration_required = true;

        let err = ContractUpgradeManager::new()
            .plan_upgrade(request)
            .unwrap_err();

        assert_eq!(
            err,
            ContractUpgradeError::InvalidStrategy(
                "migration_required cannot use in-place strategy".to_string()
            )
        );
    }

    #[test]
    fn accepts_versions_with_v_prefix() {
        let mut request = base_request();
        request.current_version = "v1.2.3".to_string();
        request.target_version = "v1.2.4".to_string();

        let plan = ContractUpgradeManager::new().plan_upgrade(request).unwrap();

        assert_eq!(plan.status, UpgradePlanStatus::Ready);
    }
}
