# Release Checklist

Use this checklist whenever cutting a new release of Crucible. The same flow works for prereleases and stable releases; the main difference is the version string and tag naming.

## 1. Decide the release type

- Stable release: use a semantic version such as `0.2.0`.
- Prerelease: use a prerelease suffix such as `0.2.0-beta.1`, `0.2.0-rc.1`, or `0.2.0-alpha.1`.
- Keep the Git tag aligned with the Cargo version and use the same version in the release notes.

## 2. Start from a clean branch

- Create a release branch from the latest `main` commit.
- Confirm the working tree is clean before making changes:

```sh
git status --short
git pull --ff-only origin main
```

## 3. Bump the workspace version

- Update the workspace version in `Cargo.toml` under `[workspace.package]`.
- Because `crucible` and `crucible-macros` inherit the workspace version, they do not need independent version edits unless a crate uses an explicit version pin.
- Review manifests for any explicit or out-of-date version references before publishing.

Example:

```toml
[workspace.package]
version = "0.2.0"
```

For prereleases, use the prerelease suffix in the same field:

```toml
[workspace.package]
version = "0.2.0-rc.1"
```

## 4. Prepare release notes

- Draft a changelog entry or release notes summary.
- Include highlights, breaking changes, new APIs, bug fixes, and any migration notes.
- Call out whether the release is a prerelease or stable release.

## 5. Verify SDK and compatibility expectations

- Confirm the supported `soroban-sdk` and Rust toolchain versions match the release goals.
- Review examples and docs for any version-specific guidance.
- If the release changes public APIs, update examples and README snippets to reflect the new behavior.

Run these checks before publishing:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --package crucible --target wasm32-unknown-unknown
```

## 6. Verify CI and release automation

- Ensure the release branch is green in CI.
- Confirm that the branch, workflow, and any release automation are configured for the intended release type.
- If the repository uses GitHub Actions for tags or releases, confirm the workflow will trigger for the planned tag.

## 7. Review docs and examples

- Update README installation snippets and any quick-start examples to match the new release.
- Check that examples still build and remain aligned with the documented API.
- Confirm any public docs or architecture notes that mention version-specific behavior are updated.

## 8. Package the crates locally

- Verify the package contents before publishing:

```sh
cargo package --allow-dirty -p crucible
cargo package --allow-dirty -p crucible-macros
```

- Verify publish dry-runs:

```sh
cargo publish --dry-run -p crucible
cargo publish --dry-run -p crucible-macros
```

## 9. Publish the crates

- Publish dependencies first, then the main crate.
- For a normal release, publish `crucible-macros` first, then `crucible`.
- For prereleases, publish the same way but keep the prerelease version suffix in the tag and release notes.

Example:

```sh
cargo publish -p crucible-macros
cargo publish -p crucible
```

## 10. Create and push the tag

- Create an annotated Git tag that matches the release version.
- Use `v` as the tag prefix for consistency.

Examples:

```sh
git tag -a v0.2.0 -m "Release v0.2.0"
```

```sh
git tag -a v0.2.0-rc.1 -m "Release v0.2.0-rc.1"
```

Push the tag:

```sh
git push origin v0.2.0
```

## 11. Post-release follow-up

- Verify the crates appear correctly on crates.io and the docs build for the new version.
- Share the release notes and tag reference with downstream users and maintainers.
- If the release was a prerelease, clearly label it as such in the GitHub release and docs.
