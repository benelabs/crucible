This PR resolves issue #886 and issue #885 for `crucible`.

### Summary of Changes

1. **Issue #886 ([79] Decentralized Bounty & Task Escrow with Milestone Payouts)**:
   - Created `crucible-example-bounty-escrow` crate at `examples/bounty-escrow`.
   - Implemented milestone lifecycle state machine (`Pending`, `Submitted`, `Approved`, `Disputed`).
   - Added proportional token payout release upon milestone approval by creator or arbiter.
   - Added dispute freezing mechanism blocking unauthorized submissions or approvals when disputed.
   - Added arbiter dispute resolution (approving payout or resetting milestone).
   - Added unreleased fund reclamation upon cancellation.
   - Added comprehensive Crucible test suite in `examples/bounty-escrow/src/test.rs` covering milestone progression, proportional payouts, dispute freezes, and dispute resolutions.

2. **Issue #885 ([78] Decentralized Peer-to-Peer Rental Protocol with Collateral Escrow)**:
   - Created `crucible-example-nft-rental` crate at `examples/nft-rental`.
   - Implemented separation of owner rights (`get_owner`) from time-bound user rights (`get_user`).
   - Implemented automatic expiration/reclamation of usability rights upon rental period expiry without modifying underlying ownership.
   - Implemented collateral escrow holding collateral during active lease periods.
   - Implemented early rental termination by renter with immediate collateral refund and revocation of usability rights.
   - Added comprehensive Crucible test suite in `examples/nft-rental/src/test.rs` verifying rights separation, automatic expiry, early termination with collateral refund, and delisting.

3. **Workspace Configuration**:
   - Added `examples/bounty-escrow` and `examples/nft-rental` to `Cargo.toml` workspace members.

Closes #886
Closes #885
