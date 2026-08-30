//! Contract gas optimization service.
//!
//! Provides analysis and optimization recommendations for smart contracts
//! to minimize gas consumption.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a single gas optimization opportunity.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Result of gas optimization analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub contract_address: String,
    pub total_opportunities: usize,
    pub total_estimated_savings: u64,
    pub opportunities: Vec<OptimizationOpportunity>,
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

        Ok(OptimizationResult {
            contract_address,
            total_opportunities: opportunities.len(),
            total_estimated_savings: total_savings,
            opportunities,
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

        Ok(OptimizationResult {
            contract_address,
            total_opportunities: opportunities.len(),
            total_estimated_savings: total_savings,
            opportunities,
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
    pub fn generate_report(&self, result: &OptimizationResult) -> String {
        let mut report = format!(
            "=== Gas Optimization Report ===\n\
            Contract: {}\n\
            Total Opportunities: {}\n\
            Estimated Total Savings: {} gas\n\n\
            Opportunities:\n",
            result.contract_address, result.total_opportunities, result.total_estimated_savings
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
}
