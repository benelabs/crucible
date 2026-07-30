This PR implements solutions for several pending infrastructure and quality issues across the repository.

### Changes Included:
- **Quality**: Established the `docs/adr/` directory for Architecture Decision Records.
- **Infrastructure**: Added a basic Prometheus exporter endpoint (`/metrics`) to `backend/src/services/metrics.rs`.
- **Infrastructure**: Added an initial ArgoCD configuration file `argocd.yaml` in the `infra/` directory to manage cluster configurations.
- **Contracts**: Optimized state structures in `contracts/src/state.rs` via bit-packing.

Closes #713
Closes #711
Closes #710
Closes #704
