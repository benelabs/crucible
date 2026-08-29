#!/bin/bash

# Script to create separate branches and PRs for the implemented features

# Use CRUCIBLE_DIR or fall back to PWD
CRUCIBLE_DIR="${CRUCIBLE_DIR:-$PWD}"
cd "$CRUCIBLE_DIR" || { echo "Error: Could not change to crucible directory at $CRUCIBLE_DIR"; exit 1; }

echo "Creating branches for the implemented features..."

echo "\n=== Issue #201: Cache Warming System ==="
git checkout -b feature/cache-warming-system
git add backend/src/workers/ backend/src/api/middleware/cache.rs backend/README.md
git commit -m "feat(workers): implement cache warming system (Issue #201)"
echo "Created branch 'feature/cache-warming-system'"

echo "\n=== Issue #197: Response Caching Middleware ==="
git checkout main
git checkout -b feature/response-caching-middleware
git add backend/src/api/middleware/cache.rs backend/src/api/middleware/mod.rs backend/README.md
git commit -m "feat(middleware): implement response caching middleware (Issue #197)"
echo "Created branch 'feature/response-caching-middleware'"

echo "\n=== Issue #188: Job Progress Tracking ==="
git checkout main
git checkout -b feature/job-progress-tracking
git add backend/src/workers/progress.rs backend/src/workers/mod.rs backend/src/workers/tests.rs backend/README.md
git commit -m "feat(workers): implement job progress tracking (Issue #188)"
echo "Created branch 'feature/job-progress-tracking'"

echo "\n=== Issue #192: Worker Health Monitoring ==="
git checkout main
git checkout -b feature/worker-health-monitoring
git add backend/src/workers/health.rs backend/src/workers/mod.rs backend/src/workers/tests.rs backend/README.md
git commit -m "feat(workers): implement worker health monitoring (Issue #192)"
echo "Created branch 'feature/worker-health-monitoring'"

echo "\n=== Summary ==="
echo "All branches created successfully!"
echo "To push and create PRs, run:"
echo "git push origin feature/cache-warming-system"
echo "git push origin feature/response-caching-middleware"
echo "git push origin feature/job-progress-tracking"
echo "git push origin feature/worker-health-monitoring"
echo "\nThen create PRs on GitHub targeting the 'main' branch."
