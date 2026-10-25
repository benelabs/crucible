//! Contract gas optimization service.
//!
//! Provides analysis and optimization recommendations for smart contracts
//! to minimize gas consumption.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a single gas optimization opportunity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizationOpportunity {
    pub id: String,
    pub optimization_type: String,
    pub description: String,
    pub estimated_gas_savings: u64,
    pub severity: OptimizationSeverity,
}

/// Severity level for optimization recommendations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OptimizationSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

use chrono::{DateTime, Utc};

/// Storage footprint estimates for persistent, temporary, and instance ledger storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StorageFootprintEstimate {
    pub persistent_storage_bytes: u64,
    pub temporary_storage_bytes: u64,
    pub instance_storage_bytes: u64,
    pub total_read_bytes: u64,
    pub total_write_bytes: u64,
}

/// Rent balance projection across ledgers for Soroban storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RentProjection {
    pub projected_rent_per_ledger: u64,
    pub estimated_monthly_rent: u64,
    pub min_rent_balance_required: u64,
    pub recommended_rent_reserve: u64,
    pub projected_ledgers_supported: u64,
}

/// Comprehensive gas and storage optimization report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GasOptimizationReport {
    pub contract_address: String,
    pub total_opportunities: usize,
    pub total_estimated_savings: u64,
    pub opportunities: Vec<OptimizationOpportunity>,
    pub storage_footprint: StorageFootprintEstimate,
    pub rent_projection: RentProjection,
    pub report: String,
}

/// Result of gas optimization analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResult {
    pub contract_address: String,
    pub total_opportunities: usize,
    pub total_estimated_savings: u64,
    pub opportunities: Vec<OptimizationOpportunity>,
    #[serde(default)]
    pub storage_footprint: Option<StorageFootprintEstimate>,
    #[serde(default)]
    pub rent_projection: Option<RentProjection>,
}

/// Real-time gas price and base fee forecaster output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GasPrediction {
    pub low_priority_fee: u64,    // 20th percentile
    pub medium_priority_fee: u64, // 50th percentile
    pub high_priority_fee: u64,   // 90th percentile
    pub base_fee: u64,
    pub ledger_count: usize,
    pub timestamp: DateTime<Utc>,
}

/// Gas optimization service for analyzing and optimizing contracts.
#[derive(Clone)]
pub struct GasOptimizer;

impl GasOptimizer {
    /// Creates a new gas optimizer instance.
    pub fn new() -> Self {
        Self
    }

    /// Analyzes recent ledger headers to compute rolling percentile gas costs and dynamic fee recommendations.
    /// Computes 20th (low), 50th (medium), and 90th (high) percentile fees across trailing ledgers.
    pub fn forecast_gas_fees(&self, recent_ledger_fees: &[u64], base_fee: u64) -> GasPrediction {
        if recent_ledger_fees.is_empty() {
            return GasPrediction {
                low_priority_fee: base_fee,
                medium_priority_fee: base_fee,
                high_priority_fee: base_fee,
                base_fee,
                ledger_count: 0,
                timestamp: Utc::now(),
            };
        }

        let mut sorted = recent_ledger_fees.to_vec();
        sorted.sort_unstable();

        let percentile = |p: f64| -> u64 {
            let index = (p * (sorted.len() - 1) as f64).round() as usize;
            sorted[index.min(sorted.len() - 1)]
        };

        GasPrediction {
            low_priority_fee: percentile(0.20),
            medium_priority_fee: percentile(0.50),
            high_priority_fee: percentile(0.90),
            base_fee,
            ledger_count: sorted.len(),
            timestamp: Utc::now(),
        }
    }

    /// Analyzes contract bytecode and returns optimization opportunities.
    ///
    /// # Arguments
    ///
    /// * `contract_address` - The address of the contract to analyze
    /// * `bytecode` - The contract bytecode/WASM to analyze
    ///
    /// # Returns
    ///
    /// A result containing optimization opportunities or an error
    pub fn analyze_bytecode(
        &self,
        contract_address: String,
        bytecode: Vec<u8>,
    ) -> Result<OptimizationResult, String> {
        if bytecode.is_empty() {
            return Err("Bytecode cannot be empty".to_string());
        }

        let mut opportunities = Vec::new();
        let mut total_savings = 0u64;

        // Check for common inefficiencies
        if self.has_unused_memory(&bytecode) {
            let savings = 5_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "unused_memory".to_string(),
                description: "Remove unused memory allocations".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Low,
            });
            total_savings += savings;
        }

        if self.has_redundant_checks(&bytecode) {
            let savings = 3_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "redundant_checks".to_string(),
                description: "Eliminate redundant validation checks".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Medium,
            });
            total_savings += savings;
        }

        if self.has_inefficient_loops(&bytecode) {
            let savings = 8_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "inefficient_loops".to_string(),
                description: "Optimize loop structures and iterations".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::High,
            });
            total_savings += savings;
        }

        if self.has_storage_ops(&bytecode) {
            let savings = 12_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "storage_optimization".to_string(),
                description: "Optimize storage access patterns".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::High,
            });
            total_savings += savings;
        }

        let storage_footprint = self.estimate_storage_footprint_from_bytecode(&bytecode);
        let rent_projection = self.project_rent_balance(&storage_footprint);

        if storage_footprint.persistent_storage_bytes > 512 {
            let savings = 15_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "storage_footprint_expansion".to_string(),
                description: "Optimize persistent storage entry sizes or migrate transient state to temporary storage to reduce rent expansion fees".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Medium,
            });
            total_savings += savings;
        }

        Ok(OptimizationResult {
            contract_address,
            total_opportunities: opportunities.len(),
            total_estimated_savings: total_savings,
            opportunities,
            storage_footprint: Some(storage_footprint),
            rent_projection: Some(rent_projection),
        })
    }

    /// Analyzes source code for optimization opportunities.
    ///
    /// # Arguments
    ///
    /// * `contract_address` - The address of the contract
    /// * `source_code` - The contract source code
    ///
    /// # Returns
    ///
    /// A result containing optimization opportunities
    pub fn analyze_source_code(
        &self,
        contract_address: String,
        source_code: &str,
    ) -> Result<OptimizationResult, String> {
        if source_code.is_empty() {
            return Err("Source code cannot be empty".to_string());
        }

        let mut opportunities = Vec::new();
        let mut total_savings = 0u64;

        // Check for common code patterns
        if source_code.contains("loop {") || source_code.contains("while") {
            let savings = 4_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "loop_optimization".to_string(),
                description: "Consider using iterators instead of loops".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Medium,
            });
            total_savings += savings;
        }

        if source_code.contains("clone()") {
            let savings = 2_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "clone_removal".to_string(),
                description: "Reduce unnecessary cloning operations".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Low,
            });
            total_savings += savings;
        }

        if source_code.contains("to_string()") || source_code.contains("format!") {
            let savings = 3_500;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "string_optimization".to_string(),
                description: "Optimize string allocations and conversions".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Low,
            });
            total_savings += savings;
        }

        let storage_footprint = self.estimate_storage_footprint_from_source(source_code);
        let rent_projection = self.project_rent_balance(&storage_footprint);

        if storage_footprint.persistent_storage_bytes > 512 {
            let savings = 15_000;
            opportunities.push(OptimizationOpportunity {
                id: Uuid::new_v4().to_string(),
                optimization_type: "storage_footprint_expansion".to_string(),
                description: "Optimize persistent storage entry sizes or migrate transient state to temporary storage to reduce rent expansion fees".to_string(),
                estimated_gas_savings: savings,
                severity: OptimizationSeverity::Medium,
            });
            total_savings += savings;
        }

        Ok(OptimizationResult {
            contract_address,
            total_opportunities: opportunities.len(),
            total_estimated_savings: total_savings,
            opportunities,
            storage_footprint: Some(storage_footprint),
            rent_projection: Some(rent_projection),
        })
    }

    /// Generates optimization report for a contract.
    ///
    /// # Arguments
    ///
    /// * `result` - The optimization analysis result
    ///
    /// # Returns
    ///
    /// A formatted report string
    /// Estimates Soroban ledger storage byte footprints based on bytecode size and storage opcodes.
    pub fn estimate_storage_footprint_from_bytecode(&self, bytecode: &[u8]) -> StorageFootprintEstimate {
        let instance_storage_bytes = 64u64;
        let persistent_storage_bytes = if self.has_storage_ops(bytecode) {
            512u64 + (bytecode.len() as u64 / 4)
        } else {
            128u64
        };
        let temporary_storage_bytes = if bytecode.contains(&0x23) {
            256u64
        } else {
            64u64
        };
        let total_read_bytes = persistent_storage_bytes + temporary_storage_bytes + instance_storage_bytes;
        let total_write_bytes = if self.has_storage_ops(bytecode) {
            persistent_storage_bytes + temporary_storage_bytes
        } else {
            64u64
        };

        StorageFootprintEstimate {
            persistent_storage_bytes,
            temporary_storage_bytes,
            instance_storage_bytes,
            total_read_bytes,
            total_write_bytes,
        }
    }

    /// Estimates Soroban ledger storage byte footprints based on contract source code patterns.
    pub fn estimate_storage_footprint_from_source(&self, source_code: &str) -> StorageFootprintEstimate {
        let instance_storage_bytes = 64u64;
        let persistent_count = source_code.matches("persistent").count() as u64;
        let temporary_count = source_code.matches("temporary").count() as u64;

        let persistent_storage_bytes = 128u64 + (persistent_count * 256u64);
        let temporary_storage_bytes = temporary_count * 128u64;
        let total_read_bytes = persistent_storage_bytes + temporary_storage_bytes + instance_storage_bytes;
        let total_write_bytes = if source_code.contains(".set(") || source_code.contains(".set_") || source_code.contains("put(") {
            persistent_storage_bytes + temporary_storage_bytes
        } else {
            64u64
        };

        StorageFootprintEstimate {
            persistent_storage_bytes,
            temporary_storage_bytes,
            instance_storage_bytes,
            total_read_bytes,
            total_write_bytes,
        }
    }

    /// Calculates projected rent balance requirements and duration based on storage footprint estimates.
    pub fn project_rent_balance(&self, footprint: &StorageFootprintEstimate) -> RentProjection {
        let projected_rent_per_ledger = ((footprint.persistent_storage_bytes * 2 + footprint.temporary_storage_bytes) / 100).max(1);
        let monthly_ledgers = 518_400u64;
        let daily_ledgers = 17_280u64;

        let estimated_monthly_rent = projected_rent_per_ledger * monthly_ledgers;
        let min_rent_balance_required = projected_rent_per_ledger * daily_ledgers;
        let recommended_rent_reserve = estimated_monthly_rent * 3;
        let projected_ledgers_supported = recommended_rent_reserve / projected_rent_per_ledger;

        RentProjection {
            projected_rent_per_ledger,
            estimated_monthly_rent,
            min_rent_balance_required,
            recommended_rent_reserve,
            projected_ledgers_supported,
        }
    }

    /// Generates structured gas and storage optimization report.
    pub fn generate_optimization_report(&self, result: &OptimizationResult) -> GasOptimizationReport {
        let footprint = result.storage_footprint.clone().unwrap_or_else(|| {
            StorageFootprintEstimate::default()
        });
        let rent = result.rent_projection.clone().unwrap_or_else(|| {
            self.project_rent_balance(&footprint)
        });
        let report_text = self.generate_report(result);

        GasOptimizationReport {
            contract_address: result.contract_address.clone(),
            total_opportunities: result.total_opportunities,
            total_estimated_savings: result.total_estimated_savings,
            opportunities: result.opportunities.clone(),
            storage_footprint: footprint,
            rent_projection: rent,
            report: report_text,
        }
    }

    pub fn generate_report(&self, result: &OptimizationResult) -> String {
        let footprint = result.storage_footprint.clone().unwrap_or_else(|| {
            StorageFootprintEstimate::default()
        });
        let rent = result.rent_projection.clone().unwrap_or_else(|| {
            self.project_rent_balance(&footprint)
        });

        let mut report = format!(
            "=== Gas Optimization Report ===\n            Contract: {}\n            Total Opportunities: {}\n            Estimated Total Savings: {} gas\n\n            === Storage Footprint Estimates ===\n            Persistent Storage: {} bytes\n            Temporary Storage: {} bytes\n            Instance Storage: {} bytes\n            Total Read Footprint: {} bytes\n            Total Write Footprint: {} bytes\n\n            === Rent Balance Projections ===\n            Projected Rent Per Ledger: {} stroops\n            Estimated Monthly Rent: {} stroops\n            Recommended Rent Reserve: {} stroops\n            Projected Ledgers Supported: {}\n\n            Opportunities:\n",
            result.contract_address,
            result.total_opportunities,
            result.total_estimated_savings,
            footprint.persistent_storage_bytes,
            footprint.temporary_storage_bytes,
            footprint.instance_storage_bytes,
            footprint.total_read_bytes,
            footprint.total_write_bytes,
            rent.projected_rent_per_ledger,
            rent.estimated_monthly_rent,
            rent.recommended_rent_reserve,
            rent.projected_ledgers_supported,
        );

        for (idx, opp) in result.opportunities.iter().enumerate() {
            report.push_str(&format!(
                "{}. [{}] {} - {} gas\n   {}\n",
                idx + 1,
                format!("{:?}", opp.severity),
                opp.optimization_type,
                opp.estimated_gas_savings,
                opp.description
            ));
        }

        report
    }

    // Private helper methods for bytecode analysis
    fn has_unused_memory(&self, bytecode: &[u8]) -> bool {
        // Check for memory allocation patterns
        bytecode.len() > 100 && bytecode.contains(&0xFF)
    }

    fn has_redundant_checks(&self, bytecode: &[u8]) -> bool {
        // Check for repeated validation patterns
        bytecode.len() > 500 && bytecode.windows(4).filter(|w| *w == [0x50, 0x50, 0x50, 0x50]).count() > 1
    }

    fn has_inefficient_loops(&self, bytecode: &[u8]) -> bool {
        // Look for loop patterns in bytecode
        bytecode.len() > 200 && bytecode.windows(2).filter(|w| *w == [0x03, 0x0B]).count() > 2
    }

    fn has_storage_ops(&self, bytecode: &[u8]) -> bool {
        // Check for storage operation patterns
        bytecode.len() > 150 && (bytecode.contains(&0x21) || bytecode.contains(&0x22))
    }
}

impl Default for GasOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_optimizer_creation() {
        let optimizer = GasOptimizer::new();
        assert!(optimizer.analyze_bytecode("0x123".to_string(), vec![]).is_err());
    }

    #[test]
    fn test_analyze_bytecode_empty() {
        let optimizer = GasOptimizer::new();
        let result = optimizer.analyze_bytecode("0x123".to_string(), vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_analyze_bytecode_success() {
        let optimizer = GasOptimizer::new();
        let bytecode = vec![0xFF, 0x50, 0x50, 0x50, 0x50, 0x03, 0x0B, 0x21];
        let result = optimizer.analyze_bytecode("0x123".to_string(), bytecode);
        
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert_eq!(analysis.contract_address, "0x123");
        assert!(analysis.total_opportunities > 0);
        assert!(analysis.total_estimated_savings > 0);
    }

    #[test]
    fn test_analyze_source_code_empty() {
        let optimizer = GasOptimizer::new();
        let result = optimizer.analyze_source_code("0x123".to_string(), "");
        assert!(result.is_err());
    }

    #[test]
    fn test_analyze_source_code_with_loops() {
        let optimizer = GasOptimizer::new();
        let source = "fn process() { loop { break; } }";
        let result = optimizer.analyze_source_code("0x456".to_string(), source);
        
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert!(analysis.total_opportunities > 0);
    }

    #[test]
    fn test_analyze_source_code_with_clones() {
        let optimizer = GasOptimizer::new();
        let source = "let a = data.clone(); let b = a.clone();";
        let result = optimizer.analyze_source_code("0x789".to_string(), source);
        
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert!(analysis.total_opportunities > 0);
    }

    #[test]
    fn test_generate_report() {
        let optimizer = GasOptimizer::new();
        let result = OptimizationResult {
            contract_address: "0xABC".to_string(),
            total_opportunities: 2,
            total_estimated_savings: 10_000,
            opportunities: vec![
                OptimizationOpportunity {
                    id: "1".to_string(),
                    optimization_type: "test".to_string(),
                    description: "test desc".to_string(),
                    estimated_gas_savings: 5_000,
                    severity: OptimizationSeverity::High,
                },
                OptimizationOpportunity {
                    id: "2".to_string(),
                    optimization_type: "test2".to_string(),
                    description: "test desc 2".to_string(),
                    estimated_gas_savings: 5_000,
                    severity: OptimizationSeverity::Medium,
                },
            ],
            storage_footprint: None,
            rent_projection: None,
        };

        let report = optimizer.generate_report(&result);
        assert!(report.contains("0xABC"));
        assert!(report.contains("10000"));
        assert!(report.contains("test"));
    }

    #[test]
    fn test_optimization_severity_serialization() {
        let severity = OptimizationSeverity::Critical;
        let json = serde_json::to_string(&severity).unwrap();
        assert_eq!(json, "\"critical\"");
    }

    #[test]
    fn test_forecast_gas_fees_percentiles() {
        let optimizer = GasOptimizer::new();
        // Fees: 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000 (10 ledgers)
        let ledger_fees: Vec<u64> = (1..=10).map(|i| i * 100).collect();
        let prediction = optimizer.forecast_gas_fees(&ledger_fees, 100);

        assert_eq!(prediction.base_fee, 100);
        assert_eq!(prediction.ledger_count, 10);
        // 20th percentile (index 0.20 * 9 = 1.8 -> 2) = 300
        assert_eq!(prediction.low_priority_fee, 300);
        // 50th percentile (index 0.50 * 9 = 4.5 -> 5) = 600
        assert_eq!(prediction.medium_priority_fee, 600);
        // 90th percentile (index 0.90 * 9 = 8.1 -> 8) = 900
        assert_eq!(prediction.high_priority_fee, 900);
    }

    #[test]
    fn test_storage_footprint_and_rent_projections() {
        let optimizer = GasOptimizer::new();
        let footprint = StorageFootprintEstimate {
            persistent_storage_bytes: 1024,
            temporary_storage_bytes: 256,
            instance_storage_bytes: 64,
            total_read_bytes: 1344,
            total_write_bytes: 1280,
        };
        let rent = optimizer.project_rent_balance(&footprint);
        assert_eq!(rent.projected_rent_per_ledger, 23);
        assert_eq!(rent.estimated_monthly_rent, 23 * 518_400);
        assert!(rent.recommended_rent_reserve > rent.min_rent_balance_required);
        assert_eq!(rent.projected_ledgers_supported, 1555200);
    }

    #[test]
    fn test_generate_optimization_report_structured() {
        let optimizer = GasOptimizer::new();
        let result = OptimizationResult {
            contract_address: "CB101".to_string(),
            total_opportunities: 1,
            total_estimated_savings: 5000,
            opportunities: vec![],
            storage_footprint: Some(StorageFootprintEstimate {
                persistent_storage_bytes: 512,
                temporary_storage_bytes: 128,
                instance_storage_bytes: 64,
                total_read_bytes: 704,
                total_write_bytes: 640,
            }),
            rent_projection: None,
        };

        let report = optimizer.generate_optimization_report(&result);
        assert_eq!(report.contract_address, "CB101");
        assert_eq!(report.storage_footprint.persistent_storage_bytes, 512);
        assert!(report.rent_projection.projected_rent_per_ledger > 0);
        assert!(report.report.contains("Storage Footprint Estimates"));
        assert!(report.report.contains("Rent Balance Projections"));
    }
}
