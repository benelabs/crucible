This PR provides comprehensive implementations resolving issues across financial smart contract governance security, microservices GraphQL Federation gateway, Redis token-bucket rate limiting middleware, and React Server Components (RSC) dashboard architecture.

### Summary of Changes

1. **[CONTRACTS] Governance Double-Voting and Infinite Inflation Vulnerability (#680)**:
   - Updated `vote` function in `contracts/governance/src/lib.rs` to query storage for existing vote record (`DataKey::Vote(voter, proposal_id)`).
   - Rejects duplicate voting attempts with `Err("Already voted")` to prevent voting power inflation.
   - Added unit test suite in `contracts/governance/src/lib.rs` verifying double-voting prevention.

2. **[BACKEND] Implement GraphQL Federation for Microservices (#683)**:
   - Implemented `GraphQLFederationGateway` in `backend/src/api/handlers/graphql.rs` unifying subgraphs across Contracts, Governance, Indexing, and Auth.
   - Added schema introspection support (`__schema`) and federated query entity resolution.
   - Registered `POST /api/v1/graphql` endpoint in `backend/src/router.rs`.

3. **[BACKEND] Redis-backed Token Bucket Rate Limiting (#684)**:
   - Implemented `TokenBucketRateLimiter` in `backend/src/api/middleware/rate_limit.rs` using Redis Lua script execution with in-memory token bucket fallback.
   - Created Axum `rate_limit_middleware` evaluating token consumption per client IP / API key and emitting `X-RateLimit-*` response headers or returning HTTP 429 (`Too Many Requests`).
   - Exported `rate_limit` in `backend/src/api/middleware/mod.rs`.

4. **[FRONTEND] React Server Components (RSC) Migration (#691)**:
   - Implemented RSC primitives and server-side data fetchers (`fetchDashboardDataServer`, `fetchEventListenerDataServer`) in `frontend/src/components/rsc/rsc.ts`.
   - Built `RscDashboard.tsx` component offloading data fetching to server components and reducing client-side bundle size.
   - Added `RscDashboard.test.tsx` testing RSC loading and data rendering.

Closes #680
Closes #683
Closes #684
Closes #691
