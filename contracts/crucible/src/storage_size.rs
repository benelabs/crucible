//! Storage size estimation utilities for Soroban contract testing.
//!
//! Provides best-effort heuristics for estimating the serialized size of
//! Soroban values so that tests can guard against exceeding the 64KB ledger
//! entry limit.
//!
//! **Host-only:** All types in this module depend on `std` and are intended
//! exclusively for use in `#[cfg(test)]` contexts on the host.

use soroban_sdk::{Env, IntoVal, xdr::ToXdr};

/// Estimates the serialized size of a value in bytes.
///
/// This is a heuristic based on typical Soroban XDR wire-format sizes.
/// For primitive types, fixed sizes are used. For composite types, the
/// estimate falls back to `std::mem::size_of_val` which provides a rough
/// upper bound.
pub fn estimate_size<T>(value: &T) -> usize {
    estimate_size_impl(value)
}

fn estimate_size_impl<T: ?Sized>(value: &T) -> usize {
    let type_name = std::any::type_name_of_val(value);

    if type_name.starts_with("soroban_sdk::String") {
        let s = value as *const T as *const soroban_sdk::String;
        let s = unsafe { &*s };
        let env = s.env();
        let val: soroban_sdk::Val = s.into_val(env);
        val.to_xdr(env).len() as usize
    } else if type_name == "soroban_sdk::Address" {
        40
    } else if type_name == "i128" || type_name == "u128" {
        16
    } else if type_name == "i64" || type_name == "u64" {
        8
    } else if type_name == "i32" || type_name == "u32" {
        4
    } else if type_name == "bool" {
        1
    } else {
        std::mem::size_of_val(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::String as SorobanString;

    #[test]
    fn test_estimate_size_bool() {
        let v = true;
        assert_eq!(estimate_size(&v), 1);
    }

    #[test]
    fn test_estimate_size_i32() {
        let v = 42i32;
        assert_eq!(estimate_size(&v), 4);
    }

    #[test]
    fn test_estimate_size_address() {
        use soroban_sdk::testutils::Address as _;
        let env = soroban_sdk::Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        assert_eq!(estimate_size(&addr), 80);
    }

    #[test]
    fn test_estimate_size_string() {
        let env = soroban_sdk::Env::default();
        let s = SorobanString::from_str(&env, "hello");
        let size = estimate_size(&s);
        assert!(size > 0);
    }

    #[test]
    fn test_assert_storage_entry_size_limit_passes_for_small_value() {
        let v: u32 = 42;
        crate::assert_storage_entry_size_limit!(v, 1024);
    }

    #[test]
    #[should_panic(expected = "assert_storage_entry_size_limit! failed")]
    fn test_assert_storage_entry_size_limit_panics_for_large_value() {
        let v: u32 = 42;
        crate::assert_storage_entry_size_limit!(v, 1);
    }
}
