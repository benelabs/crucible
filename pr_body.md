This PR introduces fixes and improvements for several issues, including:
- Adding realistic clawback support and tests for MockToken.
- Capturing failed simulations instead of panicking from `MockEnv::simulate`.
- Making `SimulatedTx` rollback state changes during dry-runs.
- Adding fixture macro tests for generic structs and where clauses.

Closes #481
Closes #491
Closes #483
Closes #482
