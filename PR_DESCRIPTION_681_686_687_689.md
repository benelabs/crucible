This PR provides comprehensive implementations resolving issues across financial smart contract authentication, asymmetric JWT key rotation and token blocklist revocation, CQRS & Event Sourcing audit trail architecture, and background worker Dead Letter Queues (DLQ).

### Summary of Changes

1. **[CONTRACTS] Oracle Whitelist Check Authorization (#681)**:
   - Modified `submit_price` in `contracts/oracle/src/lib.rs` to accept `source: Address`.
   - Added caller authentication enforcement with `source.require_auth()`.
   - Updated whitelist validation to check `source` address against `DataKey::SourceWhitelist(source)` instead of `env.current_contract_address()`.
   - Added unit test suite in `contracts/oracle/src/test.rs` verifying authorized and unauthorized price submission behaviors.

2. **[SECURITY] JWT Key Rotation and Revocation Strategy (#686)**:
   - Implemented `JwtKeyManager` (`backend/src/api/handlers/auth/jwt.rs`) managing asymmetric RSA/Ed25519 key pairs, automated key rotation (with active `kid`), and JWKS public key set distribution.
   - Implemented `TokenBlocklistService` (`backend/src/api/handlers/auth/revocation.rs`) leveraging Redis to store revoked token `jti` identifiers with TTL for instant token revocation.
   - Added auth handlers and router in `backend/src/api/handlers/auth/mod.rs` exposing `GET /api/v1/auth/jwks`, `POST /api/v1/auth/revoke`, and `POST /api/v1/auth/rotate`.
   - Added unit tests in `backend/tests/auth_tests.rs`.

3. **[BACKEND] CQRS and Event Sourcing Architecture for Audit Logs (#687)**:
   - Refactored `AuditService` in `backend/src/services/audit.rs` into a CQRS / Event Sourcing architecture.
   - **Write Path**: Appends immutable `AuditDomainEvent`s asynchronously to an event log stream / Redis PubSub channel without blocking on DB table write locks.
   - **Projection Engine**: Processes raw domain events into read-optimized projection views asynchronously.
   - **Read Path**: Executes query requests (`list_events`, `get_event`) against read-optimized projections without locking the write path.
   - Added unit tests in `backend/tests/audit_cqrs_tests.rs`.

4. **[BACKEND] Implement Dead Letter Queues (DLQ) for Async Workers (#689)**:
   - Created `DeadLetterQueue` module (`backend/src/workers/dlq.rs`) providing `enqueue`, `list`, `get`, `replay`, and `purge` operations for permanently failed jobs.
   - Exported DLQ in `backend/src/workers/mod.rs` and added unit test suite in `backend/tests/dlq_tests.rs`.

Closes #681
Closes #686
Closes #687
Closes #689
