#[derive(Debug, Clone)]
pub enum SimulationResult<T> {
    Success(T),
    Failure(SimulationError),
}

#[derive(Debug, Clone)]
pub enum SimulationError {
    AuthFailure { expected: Vec<String>, actual: Vec<String> },
    ContractError(u32),
    HostError(String),
    Panic { payload: String },
}
