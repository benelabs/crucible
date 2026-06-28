#[cfg(test)]
mod tests {
    use crate::error::{SimulationResult, SimulationError};
    use crate::simulation::SimulatedTx;

    #[test]
    fn test_simulation_panic_capture() {
        let sim: SimulatedTx<()> = SimulatedTx { 
            result: SimulationResult::Failure(SimulationError::Panic { payload: "boom".into() }),
            fee: 100,
            instructions: 10,
        };
        assert!(!sim.would_succeed());
    }
}
