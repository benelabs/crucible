use crate::error::{SimulationResult, SimulationError};

pub struct SimulatedTx<T> {
    pub result: SimulationResult<T>,
    pub fee: i64,
    pub instructions: u64,
}

impl<T> SimulatedTx<T> {
    pub fn would_succeed(&self) -> bool {
        matches!(self.result, SimulationResult::Success(_))
    }
}
