# SDK Compatibility Release Checklist

- [ ] Confirm the `soroban-sdk` version in the workspace manifest is the intended release target.
- [ ] Confirm the SDK compatibility test covers that version and run `cargo test -p crucible --test sdk_compat`.
- [ ] Verify workspace examples build against the selected SDK with `cargo check --workspace --exclude backend`.
- [ ] Record SDK compatibility changes and any required migration guidance in the release notes.