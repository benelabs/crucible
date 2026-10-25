//! Storage size estimation utilities for Soroban contract testing.
//!
//! Provides best-effort heuristics for estimating the serialized size of
//! Soroban values so that tests can guard against exceeding the 64KB ledger
//! entry limit.
//!
//! **Host-only:** All types in this module depend on `std` and are intended
//! exclusively for use in `#[cfg(test)]` contexts on the host.

use soroban_sdk::xdr::ToXdr;

// ── Public trait ─────────────────────────────────────────────────────────────

/// A type that knows how to report its own estimated serialized byte size.
///
/// Implement this for any type you want to measure via [`estimate_size`].
/// All Soroban SDK types and Rust primitives supported by crucible already
/// have implementations below; add your own for custom composite types when
/// the default `std::mem::size_of_val` fallback is not accurate enough.
pub trait StorageSize {
    /// Returns a best-effort estimate of the number of bytes this value
    /// occupies in persistent ledger storage (i.e. after XDR serialization).
    fn storage_size(&self) -> usize;
}

// ── Soroban SDK types ────────────────────────────────────────────────────────

impl StorageSize for soroban_sdk::String {
    fn storage_size(&self) -> usize {
        // `to_xdr` produces the exact on-wire representation; its byte length
        // is the true serialized size rather than the in-memory struct size.
        // No unsafe code required — the SDK exposes this natively (Soroban
        // collections report their length as `u32`).
        self.to_xdr(self.env()).len() as usize
    }
}

impl StorageSize for soroban_sdk::Address {
    /// A plain ed25519 public-key AccountID encodes to 40 bytes in XDR.
    fn storage_size(&self) -> usize {
        40
    }
}

// ── Rust primitives ──────────────────────────────────────────────────────────

impl StorageSize for i128 {
    fn storage_size(&self) -> usize {
        16
    }
}
impl StorageSize for u128 {
    fn storage_size(&self) -> usize {
        16
    }
}
impl StorageSize for i64 {
    fn storage_size(&self) -> usize {
        8
    }
}
impl StorageSize for u64 {
    fn storage_size(&self) -> usize {
        8
    }
}
impl StorageSize for i32 {
    fn storage_size(&self) -> usize {
        4
    }
}
impl StorageSize for u32 {
    fn storage_size(&self) -> usize {
        4
    }
}
impl StorageSize for bool {
    fn storage_size(&self) -> usize {
        1
    }
}

// ── Common containers ────────────────────────────────────────────────────────

/// For `Vec<T>`, use the in-memory layout size as a conservative upper bound.
/// This never reads uninitialised padding: `size_of_val` only counts the
/// allocation pointer/length/capacity header on the stack; the heap data is
/// counted via `size_of::<T>() * len`.
impl<T> StorageSize for Vec<T> {
    fn storage_size(&self) -> usize {
        // Heap payload is the meaningful part for storage estimation.
        std::mem::size_of::<T>() * self.len()
    }
}

/// For slices, count only the live elements (no padding or capacity overhead).
impl<T> StorageSize for [T] {
    fn storage_size(&self) -> usize {
        std::mem::size_of::<T>() * self.len()
    }
}

impl StorageSize for str {
    fn storage_size(&self) -> usize {
        self.len()
    }
}

impl StorageSize for String {
    fn storage_size(&self) -> usize {
        self.len()
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Estimates the serialized size of a value that implements [`StorageSize`].
///
/// Dispatch is resolved at compile time through monomorphization — no runtime
/// type inspection, no unsafe pointer casts, no UB.
///
/// For types that do **not** implement `StorageSize`, use
/// [`estimate_size_of_val`] as a coarser fallback based on
/// `std::mem::size_of_val`.
pub fn estimate_size<T: StorageSize + ?Sized>(value: &T) -> usize {
    value.storage_size()
}

/// Fallback size estimator for types that do not implement [`StorageSize`].
///
/// Returns `std::mem::size_of_val(value)`, which measures the in-memory layout
/// size.  This is a conservative upper bound: it may over-count struct padding
/// that is never written to storage.  Prefer [`estimate_size`] for any type
/// where accuracy matters.
///
/// # Note
///
/// This function does **not** read the value's bytes, so it cannot trigger UB
/// even for types with uninitialized padding.
pub fn estimate_size_of_val<T: ?Sized>(value: &T) -> usize {
    std::mem::size_of_val(value)
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
    fn test_estimate_size_u32() {
        let v = 99u32;
        assert_eq!(estimate_size(&v), 4);
    }

    #[test]
    fn test_estimate_size_i64() {
        let v = -1i64;
        assert_eq!(estimate_size(&v), 8);
    }

    #[test]
    fn test_estimate_size_u64() {
        let v = u64::MAX;
        assert_eq!(estimate_size(&v), 8);
    }

    #[test]
    fn test_estimate_size_i128() {
        let v = i128::MIN;
        assert_eq!(estimate_size(&v), 16);
    }

    #[test]
    fn test_estimate_size_u128() {
        let v = u128::MAX;
        assert_eq!(estimate_size(&v), 16);
    }

    #[test]
    fn test_estimate_size_address() {
        use soroban_sdk::testutils::Address as _;
        let env = soroban_sdk::Env::default();
        let addr = soroban_sdk::Address::generate(&env);
        assert_eq!(estimate_size(&addr), 40);
    }

    #[test]
    fn test_estimate_size_string() {
        let env = soroban_sdk::Env::default();
        let s = SorobanString::from_str(&env, "hello");
        let size = estimate_size(&s);
        assert!(size > 0, "XDR size should be positive");
    }

    #[test]
    fn test_estimate_size_of_val_fallback() {
        // An arbitrary struct not implementing StorageSize
        struct Foo {
            x: u64,
            y: u32,
        }
        let f = Foo { x: 1, y: 2 };
        // size_of_val gives the in-memory layout size (no UB, just padding-inclusive)
        assert_eq!(estimate_size_of_val(&f), std::mem::size_of::<Foo>());
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
