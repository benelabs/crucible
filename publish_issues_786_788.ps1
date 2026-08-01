# Publish branch for Issue #786 and Issue #788
git checkout -b feat/issue-786-788-full-feature-token-accessor
git add contracts/crucible/Cargo.toml
git add contracts/crucible/src/env.rs
git add .github/workflows/ci.yml
git add README.md
git add PR_DESCRIPTION_786_788.md
git add publish_issues_786_788.ps1
git add publish_issues_786_788.bat
git commit -m "feat(crucible): add 'full' feature flag (#788) and MockEnv token accessors (#786)"
git push -u origin feat/issue-786-788-full-feature-token-accessor
