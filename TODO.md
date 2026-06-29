# TODO

## backend #559 Remove duplicate std imports from backend main
- [x] Remove duplicate std imports in `backend/src/main.rs` (SocketAddr/Arc).

- [ ] Run `cargo fmt` on backend.
- [ ] Run `cargo clippy -- -D warnings` on backend.
- [ ] Ensure compilation succeeds and import duplication lint is enforced.

> Note: `cargo` could not be invoked in this environment (command not found / shell parsing errors), so formatter/clippy/compile enforcement steps are pending.

