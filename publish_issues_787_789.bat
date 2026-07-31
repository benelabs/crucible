@echo off
git checkout -b fix/issue-787-789-pool-exhaustion-fuzz-tests
git add backend/src/config/database.rs
git add backend/src/config/defaults/default.toml
git add backend/src/error.rs
git add backend/src/utils/errors.rs
git add backend/src/services/metrics.rs
git add contracts/crucible/Cargo.toml
git add contracts/crucible/src/token.rs
git add contracts/crucible/tests/property.rs
git commit -m "fix(backend,contracts): resolve #787 503 pool timeout and #789 property fuzz tests"
git push -u origin fix/issue-787-789-pool-exhaustion-fuzz-tests
