//! Compile-error and compile-success tests for the `#[fixture]` attribute macro.
//!
//! Uses [`trybuild`] to verify that the macro produces the expected errors for invalid
//! inputs and accepts valid inputs without error.
//!
//! To regenerate `.stderr` snapshot files after changing error messages, run:
//!
//! ```bash
//! TRYBUILD=overwrite cargo test -p crucible-macros
//! ```

#[test]
fn fixture_ui_tests() {
    let t = trybuild::TestCases::new();

    // --- pass cases ---
    t.pass("tests/ui/pass-basic.rs");
    t.pass("tests/ui/pass-debug-already-derived.rs");
    t.pass("tests/ui/pass-generic.rs");
    t.pass("tests/ui/pass-private-setup.rs");

    // --- fail cases ---
    t.compile_fail("tests/ui/fail-on-enum.rs");
    t.compile_fail("tests/ui/fail-on-union.rs");
    t.compile_fail("tests/ui/fail-on-fn.rs");
    t.compile_fail("tests/ui/fail-fixture-args.rs");
    t.compile_fail("tests/ui/fail-fixture-args-multiple.rs");
    t.compile_fail("tests/ui/fail-missing-setup.rs");
    t.compile_fail("tests/ui/fail-invalid-generics.rs");
}
