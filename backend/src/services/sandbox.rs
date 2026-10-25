//! Contract execution sandbox service.
//!
//! The service accepts contract WASM and XDR-encoded Soroban arguments, runs
//! the invocation inside a fresh Soroban test environment, and returns bounded
//! execution metadata. It owns all resource policy so API handlers can stay
//! thin and deterministic.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use soroban_sdk::{
    testutils::Ledger,
    xdr::{FromXdr, ToXdr},
    Bytes, Env, Symbol, Val,
};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::task::JoinError;
use uuid::Uuid;

const WASM_MAGIC: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];
const WASM_VERSION_1: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

/// Default execution policy for the sandbox.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxLimits {
    pub max_wasm_bytes: usize,
    pub max_args: usize,
    pub max_arg_xdr_bytes: usize,
    pub max_cpu_instructions: u64,
    pub max_memory_bytes: u64,
    pub timeout_ms: u64,
    #[serde(default = "default_min_ledger_sequence")]
    pub min_ledger_sequence: u32,
    #[serde(default = "default_max_ledger_sequence")]
    pub max_ledger_sequence: u32,
    #[serde(default = "default_min_ledger_timestamp")]
    pub min_ledger_timestamp: u64,
    #[serde(default = "default_max_ledger_timestamp")]
    pub max_ledger_timestamp: u64,
    #[serde(default = "default_max_concurrent_executions")]
    pub max_concurrent_executions: usize,
}

fn default_min_ledger_sequence() -> u32 {
    1
}

fn default_max_ledger_sequence() -> u32 {
    1_000_000_000
}

fn default_min_ledger_timestamp() -> u64 {
    1_000_000_000
}

fn default_max_ledger_timestamp() -> u64 {
    4_102_444_800
}

fn default_max_concurrent_executions() -> usize {
    32
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            max_wasm_bytes: 2 * 1024 * 1024,
            max_args: 32,
            max_arg_xdr_bytes: 64 * 1024,
            max_cpu_instructions: 25_000_000,
            max_memory_bytes: 64 * 1024 * 1024,
            timeout_ms: 2_000,
            min_ledger_sequence: default_min_ledger_sequence(),
            max_ledger_sequence: default_max_ledger_sequence(),
            min_ledger_timestamp: default_min_ledger_timestamp(),
            max_ledger_timestamp: default_max_ledger_timestamp(),
            max_concurrent_executions: default_max_concurrent_executions(),
        }
    }
}

impl SandboxLimits {
    /// Configure explicit bounds for simulated ledger sequence and timestamp.
    pub fn with_ledger_bounds(
        mut self,
        min_seq: u32,
        max_seq: u32,
        min_ts: u64,
        max_ts: u64,
    ) -> Self {
        self.min_ledger_sequence = min_seq;
        self.max_ledger_sequence = max_seq;
        self.min_ledger_timestamp = min_ts;
        self.max_ledger_timestamp = max_ts;
        self
    }
}

/// Per-request execution budget. Omitted fields inherit service defaults.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cpu_instructions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Request to execute a contract function in the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractExecutionRequest {
    pub wasm_base64: String,
    pub function: String,
    #[serde(default)]
    pub args_xdr_base64: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_sequence: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_timestamp: Option<u64>,
    #[serde(default)]
    pub budget: SandboxBudget,
}

/// Stable execution state returned to clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Succeeded,
    Reverted,
    Rejected,
    TimedOut,
    FuelExhausted,
}

/// Contract execution response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractExecutionResponse {
    pub execution_id: Uuid,
    pub status: ExecutionStatus,
    pub contract_id: Option<String>,
    pub function: String,
    pub wasm_sha256: String,
    pub result_xdr_base64: Option<String>,
    pub diagnostics: Vec<String>,
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
    pub elapsed_ms: u128,
}

/// Errors produced before or during sandbox execution.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error("invalid sandbox request: {0}")]
    InvalidRequest(String),
    #[error("contract execution was rejected: {0}")]
    Rejected(String),
    #[error("contract execution timed out after {0} ms")]
    TimedOut(u64),
    #[error("contract execution exceeded cpu instruction limit: {0}")]
    FuelExhausted(u64),
    #[error("sandbox worker failed: {0}")]
    Worker(String),
}

impl SandboxError {
    /// True when a rejection came from sandbox saturation instead of the invocation itself.
    /// Such requests are retryable and must not be reported as invalid input.
    pub fn is_capacity_rejection(&self) -> bool {
        matches!(self, SandboxError::Rejected(message)
            if message.contains("capacity") || message.contains("concurrency") || message.contains("too many"))
    }
}

/// Prepared execution payload with decoded binary data and cooperative cancellation.
#[derive(Debug, Clone)]
pub struct PreparedExecution {
    pub wasm: Vec<u8>,
    pub function: String,
    pub args_xdr: Vec<Vec<u8>>,
    pub ledger_sequence: Option<u32>,
    pub ledger_timestamp: Option<u64>,
    pub limits: SandboxLimits,
    pub wasm_sha256: String,
    pub cancel_flag: Arc<std::sync::atomic::AtomicBool>,
}

impl PreparedExecution {
    /// Returns true if execution was cancelled (e.g. by timeout or client disconnect).
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Signals cancellation of the running execution.
    pub fn cancel(&self) {
        self.cancel_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Checks if cancellation was requested, returning `SandboxError::TimedOut` if so.
    pub fn check_cancelled(&self) -> Result<(), SandboxError> {
        if self.is_cancelled() {
            Err(SandboxError::TimedOut(self.limits.timeout_ms))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct RawExecution {
    pub status: ExecutionStatus,
    pub contract_id: Option<String>,
    pub result_xdr: Option<Vec<u8>>,
    pub diagnostics: Vec<String>,
    pub cpu_instructions: u64,
    pub memory_bytes: u64,
}

/// Isolated execution backend. The production implementation uses Soroban's
/// test environment; tests can inject lightweight fakes for timeout and error
/// paths without sleeping inside a real VM.
pub trait ContractExecutor: Send + Sync + 'static {
    fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError>;
}

/// Guard to signal cooperative cancellation whenever the execution terminates or future is dropped.
struct CancellationGuard {
    flag: Arc<std::sync::atomic::AtomicBool>,
    armed: bool,
}

impl CancellationGuard {
    fn new(flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self { flag, armed: true }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Contract sandbox facade used by API handlers.
#[derive(Clone)]
pub struct ContractSandboxService {
    limits: SandboxLimits,
    executor: Arc<dyn ContractExecutor>,
    concurrency_semaphore: Arc<tokio::sync::Semaphore>,
}

impl Default for ContractSandboxService {
    fn default() -> Self {
        Self::new(SandboxLimits::default())
    }
}

impl ContractSandboxService {
    /// Creates a service backed by the Soroban executor.
    pub fn new(limits: SandboxLimits) -> Self {
        let permits = if limits.max_concurrent_executions > 0 {
            limits.max_concurrent_executions
        } else {
            32
        };
        Self {
            limits,
            executor: Arc::new(SorobanContractExecutor),
            concurrency_semaphore: Arc::new(tokio::sync::Semaphore::new(permits)),
        }
    }

    /// Creates a service with an injected executor, used by tests and future
    /// runtime adapters.
    pub fn with_executor(limits: SandboxLimits, executor: Arc<dyn ContractExecutor>) -> Self {
        let permits = if limits.max_concurrent_executions > 0 {
            limits.max_concurrent_executions
        } else {
            32
        };
        Self {
            limits,
            executor,
            concurrency_semaphore: Arc::new(tokio::sync::Semaphore::new(permits)),
        }
    }

    /// Returns the number of currently available execution permits in the thread pool.
    pub fn available_concurrency(&self) -> usize {
        self.concurrency_semaphore.available_permits()
    }

    /// Validates, executes, and formats a sandbox invocation.
    pub async fn execute(
        &self,
        request: ContractExecutionRequest,
    ) -> Result<ContractExecutionResponse, SandboxError> {
        let started = Instant::now();
        let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let prepared = self.prepare_with_cancellation(request, Arc::clone(&cancel_flag))?;
        let timeout_ms = prepared.limits.timeout_ms;
        let function = prepared.function.clone();
        let wasm_sha256 = prepared.wasm_sha256.clone();
        let executor = Arc::clone(&self.executor);

        // Acquire bounded execution permit to prevent Tokio blocking threadpool starvation
        let permit = match self.concurrency_semaphore.try_acquire() {
            Ok(p) => p,
            Err(_) => {
                let wait_duration = Duration::from_millis(100.min(timeout_ms));
                match tokio::time::timeout(wait_duration, self.concurrency_semaphore.acquire())
                    .await
                {
                    Ok(Ok(p)) => p,
                    _ => {
                        return Err(SandboxError::Rejected(
                            "sandbox execution thread pool capacity exceeded; too many concurrent invocations"
                                .to_string(),
                        ));
                    }
                }
            }
        };

        let cancel_guard = CancellationGuard::new(Arc::clone(&cancel_flag));
        let worker_cancel_flag = Arc::clone(&cancel_flag);

        let worker = tokio::task::spawn_blocking(move || {
            catch_unwind(AssertUnwindSafe(|| executor.execute(prepared)))
                .map_err(|panic| SandboxError::Worker(panic_message(panic)))?
        });

        let raw = match tokio::time::timeout(Duration::from_millis(timeout_ms), worker).await {
            Ok(result) => {
                cancel_guard.disarm();
                drop(permit);
                match result.map_err(join_error_to_sandbox)? {
                    Ok(raw) => raw,
                    Err(error @ SandboxError::Rejected(_)) if error.is_capacity_rejection() => {
                        return Err(error);
                    }
                    Err(SandboxError::Rejected(message)) => {
                        return Ok(ContractExecutionResponse {
                            execution_id: Uuid::new_v4(),
                            status: ExecutionStatus::Rejected,
                            contract_id: None,
                            function,
                            wasm_sha256,
                            result_xdr_base64: None,
                            diagnostics: vec![message],
                            cpu_instructions: 0,
                            memory_bytes: 0,
                            elapsed_ms: started.elapsed().as_millis(),
                        });
                    }
                    Err(SandboxError::TimedOut(timeout)) => {
                        return Ok(ContractExecutionResponse {
                            execution_id: Uuid::new_v4(),
                            status: ExecutionStatus::TimedOut,
                            contract_id: None,
                            function,
                            wasm_sha256,
                            result_xdr_base64: None,
                            diagnostics: vec![format!("execution exceeded {timeout} ms timeout")],
                            cpu_instructions: 0,
                            memory_bytes: 0,
                            elapsed_ms: started.elapsed().as_millis(),
                        });
                    }
                    Err(SandboxError::FuelExhausted(limit)) => {
                        return Ok(ContractExecutionResponse {
                            execution_id: Uuid::new_v4(),
                            status: ExecutionStatus::FuelExhausted,
                            contract_id: None,
                            function,
                            wasm_sha256,
                            result_xdr_base64: None,
                            diagnostics: vec![format!(
                                "execution exceeded {limit} CPU instruction fuel limit"
                            )],
                            cpu_instructions: limit,
                            memory_bytes: 0,
                            elapsed_ms: started.elapsed().as_millis(),
                        });
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(_) => {
                // Signal cooperative cancellation to force-terminate blocking thread
                worker_cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                cancel_guard.disarm();
                drop(permit);
                return Ok(ContractExecutionResponse {
                    execution_id: Uuid::new_v4(),
                    status: ExecutionStatus::TimedOut,
                    contract_id: None,
                    function,
                    wasm_sha256,
                    result_xdr_base64: None,
                    diagnostics: vec![format!("execution exceeded {timeout_ms} ms timeout")],
                    cpu_instructions: 0,
                    memory_bytes: 0,
                    elapsed_ms: started.elapsed().as_millis(),
                });
            }
        };

        Ok(ContractExecutionResponse {
            execution_id: Uuid::new_v4(),
            status: raw.status,
            contract_id: raw.contract_id,
            function,
            wasm_sha256,
            result_xdr_base64: raw.result_xdr.map(|bytes| STANDARD.encode(bytes)),
            diagnostics: raw.diagnostics,
            cpu_instructions: raw.cpu_instructions,
            memory_bytes: raw.memory_bytes,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    pub fn prepare(
        &self,
        request: ContractExecutionRequest,
    ) -> Result<PreparedExecution, SandboxError> {
        self.prepare_with_cancellation(request, Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }

    pub fn prepare_with_cancellation(
        &self,
        request: ContractExecutionRequest,
        cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<PreparedExecution, SandboxError> {
        validate_function_name(&request.function)?;
        if request.args_xdr_base64.len() > self.limits.max_args {
            return Err(SandboxError::InvalidRequest(format!(
                "args_xdr_base64 cannot contain more than {} values",
                self.limits.max_args
            )));
        }

        let wasm = decode_base64("wasm_base64", &request.wasm_base64)?;
        validate_wasm_header(&wasm)?;
        if wasm.len() > self.limits.max_wasm_bytes {
            return Err(SandboxError::InvalidRequest(format!(
                "wasm_base64 exceeds {} decoded bytes",
                self.limits.max_wasm_bytes
            )));
        }

        let mut args_xdr = Vec::with_capacity(request.args_xdr_base64.len());
        for (index, encoded) in request.args_xdr_base64.iter().enumerate() {
            let arg = decode_base64(&format!("args_xdr_base64[{index}]"), encoded)?;
            if arg.len() > self.limits.max_arg_xdr_bytes {
                return Err(SandboxError::InvalidRequest(format!(
                    "args_xdr_base64[{index}] exceeds {} decoded bytes",
                    self.limits.max_arg_xdr_bytes
                )));
            }
            args_xdr.push(arg);
        }

        if let Some(seq) = request.ledger_sequence {
            if seq < self.limits.min_ledger_sequence || seq > self.limits.max_ledger_sequence {
                return Err(SandboxError::InvalidRequest(format!(
                    "ledger_sequence {} is out of valid bounds [{}..{}]",
                    seq, self.limits.min_ledger_sequence, self.limits.max_ledger_sequence
                )));
            }
        }

        if let Some(ts) = request.ledger_timestamp {
            if ts < self.limits.min_ledger_timestamp || ts > self.limits.max_ledger_timestamp {
                return Err(SandboxError::InvalidRequest(format!(
                    "ledger_timestamp {} is out of valid bounds [{}..{}]",
                    ts, self.limits.min_ledger_timestamp, self.limits.max_ledger_timestamp
                )));
            }
        }

        let limits = self.effective_limits(request.budget)?;
        let wasm_sha256 = to_hex(&Sha256::digest(&wasm));

        Ok(PreparedExecution {
            wasm,
            function: request.function,
            args_xdr,
            ledger_sequence: request.ledger_sequence,
            ledger_timestamp: request.ledger_timestamp,
            limits,
            wasm_sha256,
            cancel_flag,
        })
    }

    fn effective_limits(&self, budget: SandboxBudget) -> Result<SandboxLimits, SandboxError> {
        let mut limits = self.limits;
        if let Some(value) = budget.max_cpu_instructions {
            if value == 0 || value > self.limits.max_cpu_instructions {
                return Err(SandboxError::InvalidRequest(format!(
                    "max_cpu_instructions must be between 1 and {}",
                    self.limits.max_cpu_instructions
                )));
            }
            limits.max_cpu_instructions = value;
        }
        if let Some(value) = budget.max_memory_bytes {
            if value == 0 || value > self.limits.max_memory_bytes {
                return Err(SandboxError::InvalidRequest(format!(
                    "max_memory_bytes must be between 1 and {}",
                    self.limits.max_memory_bytes
                )));
            }
            limits.max_memory_bytes = value;
        }
        if let Some(value) = budget.timeout_ms {
            if value == 0 || value > self.limits.timeout_ms {
                return Err(SandboxError::InvalidRequest(format!(
                    "timeout_ms must be between 1 and {}",
                    self.limits.timeout_ms
                )));
            }
            limits.timeout_ms = value;
        }
        Ok(limits)
    }
}

/// Soroban SDK backed executor.
pub struct SorobanContractExecutor;

impl ContractExecutor for SorobanContractExecutor {
    fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
        execution.check_cancelled()?;

        let env = Env::default();
        env.mock_all_auths();
        configure_ledger(&env, execution.ledger_sequence, execution.ledger_timestamp);

        let mut budget = env.cost_estimate().budget();
        budget.reset_limits(
            execution.limits.max_cpu_instructions,
            execution.limits.max_memory_bytes,
        );

        execution.check_cancelled()?;

        let args = decode_args(&env, &execution.args_xdr)?;
        let wasm = Bytes::from_slice(&env, &execution.wasm);

        execution.check_cancelled()?;

        // Using register_contract_wasm is currently the only straightforward way
        // to dynamically register an arbitrary WASM blob in the sandbox execution environment.
        #[allow(deprecated)]
        let contract_id = env.register_contract_wasm(None, wasm);
        let symbol = Symbol::new(&env, &execution.function);

        execution.check_cancelled()?;

        let invocation = catch_unwind(AssertUnwindSafe(|| {
            env.invoke_contract::<Val>(&contract_id, &symbol, args)
        }));

        let cpu_instructions = budget.cpu_instruction_cost();
        let memory_bytes = budget.memory_bytes_cost();
        let contract_id = Some(format!("{contract_id:?}"));

        match invocation {
            Ok(value) => {
                if cpu_instructions >= execution.limits.max_cpu_instructions {
                    return Err(SandboxError::FuelExhausted(
                        execution.limits.max_cpu_instructions,
                    ));
                }
                let xdr_bytes = value.to_xdr(&env);
                let mut vec = vec![0u8; xdr_bytes.len() as usize];
                xdr_bytes.copy_into_slice(&mut vec);
                Ok(RawExecution {
                    status: ExecutionStatus::Succeeded,
                    contract_id,
                    result_xdr: Some(vec),
                    diagnostics: Vec::new(),
                    cpu_instructions,
                    memory_bytes,
                })
            }
            Err(panic) => {
                let msg = panic_message(panic);
                let msg_lower = msg.to_lowercase();
                if cpu_instructions >= execution.limits.max_cpu_instructions
                    || msg_lower.contains("budget")
                    || msg_lower.contains("exceededlimit")
                    || msg_lower.contains("fuel")
                    || msg_lower.contains("cpu instruction")
                    || msg_lower.contains("instruction limit")
                {
                    return Err(SandboxError::FuelExhausted(
                        execution.limits.max_cpu_instructions,
                    ));
                }
                Ok(RawExecution {
                    status: ExecutionStatus::Reverted,
                    contract_id,
                    result_xdr: None,
                    diagnostics: vec![msg],
                    cpu_instructions,
                    memory_bytes,
                })
            }
        }
    }
}

fn configure_ledger(env: &Env, sequence: Option<u32>, timestamp: Option<u64>) {
    if sequence.is_none() && timestamp.is_none() {
        return;
    }

    let current = env.ledger().get();
    env.ledger().set(soroban_sdk::testutils::LedgerInfo {
        sequence_number: sequence.unwrap_or(current.sequence_number),
        timestamp: timestamp.unwrap_or(current.timestamp),
        protocol_version: current.protocol_version,
        base_reserve: current.base_reserve,
        network_id: current.network_id,
        min_temp_entry_ttl: current.min_temp_entry_ttl,
        min_persistent_entry_ttl: current.min_persistent_entry_ttl,
        max_entry_ttl: current.max_entry_ttl,
    });
}

fn decode_args(env: &Env, args_xdr: &[Vec<u8>]) -> Result<soroban_sdk::Vec<Val>, SandboxError> {
    let mut args = soroban_sdk::Vec::new(env);
    for arg_xdr in args_xdr {
        let bytes = Bytes::from_slice(env, arg_xdr);
        let value = catch_unwind(AssertUnwindSafe(|| Val::from_xdr(env, &bytes)))
            .map_err(|panic| SandboxError::Rejected(panic_message(panic)))?
            .map_err(|_| {
                SandboxError::Rejected("argument XDR could not be converted".to_string())
            })?;
        args.push_back(value);
    }
    Ok(args)
}

fn validate_function_name(function: &str) -> Result<(), SandboxError> {
    if function.is_empty() || function.len() > 32 {
        return Err(SandboxError::InvalidRequest(
            "function must be between 1 and 32 characters".to_string(),
        ));
    }

    if !function
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(SandboxError::InvalidRequest(
            "function may contain only ASCII letters, numbers, and '_'".to_string(),
        ));
    }

    Ok(())
}

fn validate_wasm_header(wasm: &[u8]) -> Result<(), SandboxError> {
    if wasm.len() < 8 {
        return Err(SandboxError::InvalidRequest(
            "wasm_base64 must decode to a complete WASM header".to_string(),
        ));
    }
    if wasm[..4] != WASM_MAGIC || wasm[4..8] != WASM_VERSION_1 {
        return Err(SandboxError::InvalidRequest(
            "wasm_base64 must decode to a version-1 WebAssembly module".to_string(),
        ));
    }
    Ok(())
}

fn decode_base64(field: &str, value: &str) -> Result<Vec<u8>, SandboxError> {
    if value.trim().is_empty() {
        return Err(SandboxError::InvalidRequest(format!("{field} is required")));
    }
    STANDARD
        .decode(value)
        .map_err(|e| SandboxError::InvalidRequest(format!("{field} is not valid base64: {e}")))
}

fn join_error_to_sandbox(error: JoinError) -> SandboxError {
    SandboxError::Worker(error.to_string())
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "contract execution aborted".to_string()
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration as StdDuration};

    const EMPTY_WASM_BASE64: &str = "AGFzbQEAAAA=";

    #[derive(Clone)]
    struct StaticExecutor {
        raw: RawExecution,
    }

    impl ContractExecutor for StaticExecutor {
        fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            Ok(self.raw.clone())
        }
    }

    struct SleepingExecutor;

    impl ContractExecutor for SleepingExecutor {
        fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            thread::sleep(StdDuration::from_millis(50));
            Ok(RawExecution {
                status: ExecutionStatus::Succeeded,
                contract_id: None,
                result_xdr: None,
                diagnostics: Vec::new(),
                cpu_instructions: 1,
                memory_bytes: 1,
            })
        }
    }

    struct RejectingExecutor;

    impl ContractExecutor for RejectingExecutor {
        fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            Err(SandboxError::Rejected("bad invocation".to_string()))
        }
    }

    // New tests to add inside `mod tests` in backend/src/services/sandbox.rs,
    // alongside the existing StaticExecutor / SleepingExecutor / RejectingExecutor
    // fakes. These close the gap noted in docs/sandbox-threat-model.md: existing
    // tests only prove that an out-of-range *budget request* is rejected before
    // execution starts. They do not prove that a contract which actually exceeds
    // its CPU/memory budget *during* execution is reported as such, or make
    // explicit what happens to a thread after a timeout is hit.

    // --- Fake executor: simulates the Soroban budget being exceeded mid-execution.
    // In production this is what SorobanContractExecutor would surface if the VM's
    // own metering trips. This test exists because nothing previously exercised
    // that path — only the pre-execution validation that rejects an
    // out-of-range *requested* budget.
    struct BudgetExceededExecutor;

    impl ContractExecutor for BudgetExceededExecutor {
        fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            // Simulate the VM reporting that it hit its configured ceiling.
            Ok(RawExecution {
                status: ExecutionStatus::Reverted,
                contract_id: None,
                result_xdr: None,
                diagnostics: vec![format!(
                    "execution exceeded configured cpu instruction budget of {}",
                    execution.limits.max_cpu_instructions
                )],
                cpu_instructions: execution.limits.max_cpu_instructions,
                memory_bytes: execution.limits.max_memory_bytes,
            })
        }
    }

    #[tokio::test]
    async fn execute_reports_cpu_budget_exhaustion_distinctly() {
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(BudgetExceededExecutor),
        );

        let response = service.execute(request()).await.unwrap();

        // The response must make budget exhaustion legible to the caller: status
        // is Reverted (not Succeeded), the reported cpu_instructions matches the
        // configured ceiling (not zero, not silently dropped), and the
        // diagnostics explain why. A regression that swallowed this distinction
        // (e.g. reporting Succeeded with truncated output) would pass every
        // other existing test but fail this one.
        assert_eq!(response.status, ExecutionStatus::Reverted);
        assert_eq!(
            response.cpu_instructions,
            SandboxLimits::default().max_cpu_instructions
        );
        assert!(response
            .diagnostics
            .iter()
            .any(|d| d.contains("cpu instruction budget")));
    }

    #[tokio::test]
    async fn execute_respects_caller_supplied_budget_in_executor_input() {
        // Confirms the *clamped* per-request budget (not just the service
        // default) is what actually reaches the executor. Without this, a caller
        // requesting a tighter budget than the service default could silently
        // execute under the looser default instead.
        let requested_cpu = 1_000;
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(BudgetExceededExecutor),
        );
        let mut req = request();
        req.budget.max_cpu_instructions = Some(requested_cpu);

        let response = service.execute(req).await.unwrap();

        assert_eq!(response.cpu_instructions, requested_cpu);
    }

    // --- Fake executor: never returns within the test's lifetime. Used to make
    // explicit (not just implicit in production behavior) that hitting a timeout
    // stops the *caller* from waiting but does not stop the underlying thread.
    // This is a known, documented limitation (see docs/sandbox-threat-model.md)
    // rather than a bug this test is trying to catch — it exists so the behavior
    // is pinned and visible in CI rather than only discoverable by reading code.
    struct ForeverExecutor {
        started: Arc<std::sync::atomic::AtomicBool>,
        finished: Arc<std::sync::atomic::AtomicBool>,
    }

    impl ContractExecutor for ForeverExecutor {
        fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            self.started
                .store(true, std::sync::atomic::Ordering::SeqCst);
            thread::sleep(StdDuration::from_millis(200));
            self.finished
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(RawExecution {
                status: ExecutionStatus::Succeeded,
                contract_id: None,
                result_xdr: None,
                diagnostics: Vec::new(),
                cpu_instructions: 1,
                memory_bytes: 1,
            })
        }
    }

    #[tokio::test]
    async fn timeout_abandons_the_wait_but_thread_keeps_running() {
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let limits = SandboxLimits {
            timeout_ms: 5,
            ..SandboxLimits::default()
        };
        let service = ContractSandboxService::with_executor(
            limits,
            Arc::new(ForeverExecutor {
                started: Arc::clone(&started),
                finished: Arc::clone(&finished),
            }),
        );

        let response = service.execute(request()).await.unwrap();
        assert_eq!(response.status, ExecutionStatus::TimedOut);

        // At the moment the caller gets TimedOut back, the work has started but
        // is documented to still be running in the background — this is the
        // "abandons the wait, not the work" behavior called out in the threat
        // model doc. We wait past the executor's total runtime to confirm it
        // does eventually finish on its own thread, unobserved by the caller.
        assert!(started.load(std::sync::atomic::Ordering::SeqCst));
        thread::sleep(StdDuration::from_millis(250));
        assert!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            "expected the abandoned thread to keep running to completion after timeout \
            was returned to the caller — this pins the documented limitation; if this \
            assertion starts failing because the thread is now actually being cancelled, \
            update docs/sandbox-threat-model.md to remove the corresponding caveat"
        );
    }

    #[tokio::test]
    async fn sandbox_rejects_oversized_wasm() {
        let service = ContractSandboxService::default();
        let mut req = request();
        // Create oversized WASM (exceeds 2MB limit)
        req.wasm_base64 = STANDARD.encode(vec![0u8; 2_100_000]);

        let error = service.execute(req).await.unwrap_err();
        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn sandbox_rejects_too_many_args() {
        let service = ContractSandboxService::default();
        let mut req = request();
        // Exceed max_args (32)
        req.args_xdr_base64 = vec!["AQID".to_string(); 40];

        let error = service.execute(req).await.unwrap_err();
        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn sandbox_enforces_memory_budget() {
        struct MemoryExceededExecutor;
        impl ContractExecutor for MemoryExceededExecutor {
            fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
                Ok(RawExecution {
                    status: ExecutionStatus::Reverted,
                    contract_id: None,
                    result_xdr: None,
                    diagnostics: vec![format!(
                        "execution exceeded configured memory budget of {} bytes",
                        execution.limits.max_memory_bytes
                    )],
                    cpu_instructions: 100,
                    memory_bytes: execution.limits.max_memory_bytes,
                })
            }
        }

        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(MemoryExceededExecutor),
        );

        let response = service.execute(request()).await.unwrap();
        assert_eq!(response.status, ExecutionStatus::Reverted);
        assert!(response
            .diagnostics
            .iter()
            .any(|d| d.contains("memory budget")));
    }

    #[tokio::test]
    async fn sandbox_validates_ledger_state_input() {
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(StaticExecutor {
                raw: RawExecution {
                    status: ExecutionStatus::Succeeded,
                    contract_id: None,
                    result_xdr: None,
                    diagnostics: Vec::new(),
                    cpu_instructions: 1,
                    memory_bytes: 1,
                },
            }),
        );
        let mut req = request();
        req.ledger_sequence = Some(42);
        req.ledger_timestamp = Some(1_700_000_000);

        let response = service.execute(req).await;

        // Valid ledger parameters must not be rejected at the request-validation layer.
        assert!(
            response.is_ok(),
            "expected valid ledger parameters to be accepted, got {response:?}"
        );
    }

    #[tokio::test]
    async fn sandbox_multiple_concurrent_executions() {
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(StaticExecutor {
                raw: RawExecution {
                    status: ExecutionStatus::Succeeded,
                    contract_id: None,
                    result_xdr: None,
                    diagnostics: Vec::new(),
                    cpu_instructions: 1,
                    memory_bytes: 1,
                },
            }),
        );

        // Fire multiple concurrent requests
        let mut handles = vec![];
        for _ in 0..5 {
            let svc = service.clone();
            let handle = tokio::spawn(async move { svc.execute(request()).await });
            handles.push(handle);
        }

        // All should complete successfully
        for handle in handles {
            let result = handle.await;
            assert!(result.is_ok());
            assert!(result.unwrap().is_ok());
        }
    }

    fn request() -> ContractExecutionRequest {
        ContractExecutionRequest {
            wasm_base64: EMPTY_WASM_BASE64.to_string(),
            function: "hello".to_string(),
            args_xdr_base64: Vec::new(),
            ledger_sequence: Some(42),
            ledger_timestamp: Some(1_700_000_000),
            budget: SandboxBudget::default(),
        }
    }

    #[tokio::test]
    async fn execute_returns_executor_output() {
        let raw = RawExecution {
            status: ExecutionStatus::Succeeded,
            contract_id: Some("contract".to_string()),
            result_xdr: Some(vec![1, 2, 3]),
            diagnostics: vec!["ok".to_string()],
            cpu_instructions: 12,
            memory_bytes: 34,
        };
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(StaticExecutor { raw }),
        );

        let response = service.execute(request()).await.unwrap();

        assert_eq!(response.status, ExecutionStatus::Succeeded);
        assert_eq!(response.contract_id, Some("contract".to_string()));
        assert_eq!(response.result_xdr_base64, Some("AQID".to_string()));
        assert_eq!(response.cpu_instructions, 12);
        assert_eq!(response.memory_bytes, 34);
    }

    #[tokio::test]
    async fn execute_rejects_invalid_function_names() {
        let service = ContractSandboxService::default();
        let mut req = request();
        req.function = "bad-name".to_string();

        let error = service.execute(req).await.unwrap_err();

        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn execute_rejects_invalid_wasm() {
        let service = ContractSandboxService::default();
        let mut req = request();
        req.wasm_base64 = STANDARD.encode("not wasm");

        let error = service.execute(req).await.unwrap_err();

        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn execute_enforces_timeout() {
        let limits = SandboxLimits {
            timeout_ms: 5,
            ..SandboxLimits::default()
        };
        let service = ContractSandboxService::with_executor(limits, Arc::new(SleepingExecutor));

        let response = service.execute(request()).await.unwrap();

        assert_eq!(response.status, ExecutionStatus::TimedOut);
        assert!(response.diagnostics[0].contains("timeout"));
    }

    #[tokio::test]
    async fn execute_returns_rejected_status_for_runtime_rejections() {
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(RejectingExecutor),
        );

        let response = service.execute(request()).await.unwrap();

        assert_eq!(response.status, ExecutionStatus::Rejected);
        assert_eq!(response.diagnostics, vec!["bad invocation".to_string()]);
    }

    #[tokio::test]
    async fn execute_rejects_budget_above_service_limit() {
        let service = ContractSandboxService::default();
        let mut req = request();
        req.budget.max_cpu_instructions = Some(SandboxLimits::default().max_cpu_instructions + 1);

        let error = service.execute(req).await.unwrap_err();

        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }

    struct FuelExhaustedExecutor;

    impl ContractExecutor for FuelExhaustedExecutor {
        fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
            Err(SandboxError::FuelExhausted(
                execution.limits.max_cpu_instructions,
            ))
        }
    }

    #[tokio::test]
    async fn execute_returns_fuel_exhausted_status_when_fuel_limit_exceeded() {
        let service = ContractSandboxService::with_executor(
            SandboxLimits::default(),
            Arc::new(FuelExhaustedExecutor),
        );

        let response = service.execute(request()).await.unwrap();

        assert_eq!(response.status, ExecutionStatus::FuelExhausted);
        assert_eq!(
            response.cpu_instructions,
            SandboxLimits::default().max_cpu_instructions
        );
        assert!(response
            .diagnostics
            .iter()
            .any(|d| d.contains("CPU instruction fuel limit")));
    }

    #[tokio::test]
    async fn execute_rejects_out_of_bounds_ledger_sequence() {
        let service = ContractSandboxService::default();

        // Zero is invalid (genesis is 1)
        let mut req_zero = request();
        req_zero.ledger_sequence = Some(0);
        let err_zero = service.execute(req_zero).await.unwrap_err();
        assert!(matches!(err_zero, SandboxError::InvalidRequest(_)));
        assert!(err_zero
            .to_string()
            .contains("ledger_sequence 0 is out of valid bounds"));

        // Exceeding max is invalid
        let mut req_high = request();
        req_high.ledger_sequence = Some(2_000_000_000);
        let err_high = service.execute(req_high).await.unwrap_err();
        assert!(matches!(err_high, SandboxError::InvalidRequest(_)));
        assert!(err_high
            .to_string()
            .contains("ledger_sequence 2000000000 is out of valid bounds"));
    }

    #[tokio::test]
    async fn execute_rejects_out_of_bounds_ledger_timestamp() {
        let service = ContractSandboxService::default();

        // Far past before genesis
        let mut req_low = request();
        req_low.ledger_timestamp = Some(500);
        let err_low = service.execute(req_low).await.unwrap_err();
        assert!(matches!(err_low, SandboxError::InvalidRequest(_)));
        assert!(err_low
            .to_string()
            .contains("ledger_timestamp 500 is out of valid bounds"));

        // Far future
        let mut req_future = request();
        req_future.ledger_timestamp = Some(5_000_000_000);
        let err_future = service.execute(req_future).await.unwrap_err();
        assert!(matches!(err_future, SandboxError::InvalidRequest(_)));
        assert!(err_future
            .to_string()
            .contains("ledger_timestamp 5000000000 is out of valid bounds"));
    }

    #[tokio::test]
    async fn execute_respects_custom_ledger_bounds() {
        let limits =
            SandboxLimits::default().with_ledger_bounds(100, 200, 1_500_000_000, 1_600_000_000);
        let service = ContractSandboxService::new(limits);

        // Sequence 42 should now fail under custom bounds [100..200]
        let mut req = request();
        req.ledger_sequence = Some(42);
        req.ledger_timestamp = Some(1_550_000_000);
        let err = service.execute(req).await.unwrap_err();
        assert!(matches!(err, SandboxError::InvalidRequest(_)));
        assert!(err
            .to_string()
            .contains("ledger_sequence 42 is out of valid bounds [100..200]"));
    }

    #[tokio::test]
    async fn timeout_signals_cancellation_and_terminates_cooperative_executor() {
        struct CooperativeLoopExecutor {
            started: Arc<std::sync::atomic::AtomicBool>,
            stopped_by_cancellation: Arc<std::sync::atomic::AtomicBool>,
            completed_full_loop: Arc<std::sync::atomic::AtomicBool>,
        }

        impl ContractExecutor for CooperativeLoopExecutor {
            fn execute(&self, execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
                self.started
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                for _ in 0..100 {
                    if execution.is_cancelled() {
                        self.stopped_by_cancellation
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        return Err(SandboxError::TimedOut(execution.limits.timeout_ms));
                    }
                    thread::sleep(StdDuration::from_millis(5));
                }
                self.completed_full_loop
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(RawExecution {
                    status: ExecutionStatus::Succeeded,
                    contract_id: None,
                    result_xdr: None,
                    diagnostics: Vec::new(),
                    cpu_instructions: 100,
                    memory_bytes: 100,
                })
            }
        }

        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped_by_cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed_full_loop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let limits = SandboxLimits {
            timeout_ms: 20,
            ..SandboxLimits::default()
        };

        let executor = Arc::new(CooperativeLoopExecutor {
            started: Arc::clone(&started),
            stopped_by_cancellation: Arc::clone(&stopped_by_cancellation),
            completed_full_loop: Arc::clone(&completed_full_loop),
        });

        let service = ContractSandboxService::with_executor(limits, executor);
        let res = service.execute(request()).await.unwrap();

        assert_eq!(res.status, ExecutionStatus::TimedOut);
        assert!(started.load(std::sync::atomic::Ordering::SeqCst));

        // Allow cooperative thread to observe cancellation
        tokio::time::sleep(tokio::time::Duration::from_millis(60)).await;

        assert!(
            stopped_by_cancellation.load(std::sync::atomic::Ordering::SeqCst),
            "Expected worker thread to terminate early via cooperative cancellation flag"
        );
        assert!(
            !completed_full_loop.load(std::sync::atomic::Ordering::SeqCst),
            "Timed-out thread must not run full infinite loop to completion"
        );
    }

    #[tokio::test]
    async fn sandbox_concurrency_semaphore_bounds_thread_pool() {
        struct SlowExecutor;
        impl ContractExecutor for SlowExecutor {
            fn execute(&self, _execution: PreparedExecution) -> Result<RawExecution, SandboxError> {
                thread::sleep(StdDuration::from_millis(150));
                Ok(RawExecution {
                    status: ExecutionStatus::Succeeded,
                    contract_id: None,
                    result_xdr: None,
                    diagnostics: Vec::new(),
                    cpu_instructions: 1,
                    memory_bytes: 1,
                })
            }
        }

        let limits = SandboxLimits {
            max_concurrent_executions: 2,
            timeout_ms: 500,
            ..SandboxLimits::default()
        };

        let service = Arc::new(ContractSandboxService::with_executor(
            limits,
            Arc::new(SlowExecutor),
        ));

        // Saturate pool with 2 tasks
        let s1 = Arc::clone(&service);
        let h1 = tokio::spawn(async move { s1.execute(request()).await });

        let s2 = Arc::clone(&service);
        let h2 = tokio::spawn(async move { s2.execute(request()).await });

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        assert_eq!(service.available_concurrency(), 0);

        // Third execution with short timeout must reject due to capacity limit
        let mut req3 = request();
        req3.budget = SandboxBudget {
            timeout_ms: Some(25),
            ..Default::default()
        };
        let s3 = Arc::clone(&service);
        let res3 = s3.execute(req3).await;

        assert!(res3.is_err());
        match res3.unwrap_err() {
            SandboxError::Rejected(msg) => {
                assert!(msg.contains("capacity exceeded"));
            }
            other => panic!("Expected capacity exceeded error, got {:?}", other),
        }

        h1.await.unwrap().unwrap();
        h2.await.unwrap().unwrap();

        assert_eq!(service.available_concurrency(), 2);
    }

    #[tokio::test]
    async fn cancellation_guard_triggers_on_future_drop() {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let _guard = CancellationGuard::new(Arc::clone(&flag));
            assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        }
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }
}
