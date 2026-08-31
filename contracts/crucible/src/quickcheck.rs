//! Property-based and differential fuzzing runtime for Soroban contracts.
//!
//! This module is the runtime half of the
//! [`#[crucible::quickcheck]`][macro@crate::quickcheck] attribute macro. Tests
//! normally use the macro and never name anything here directly, but the
//! pieces are public so a bespoke harness can reuse them.
//!
//! # What the macro does
//!
//! Given a test function whose parameters implement [`Arbitrary`], the macro
//! generates a `#[test]` that runs the body against many generated inputs and,
//! on failure, **shrinks** the input to a minimal reproducing case before
//! reporting it.
//!
//! ```rust,ignore
//! use crucible::prelude::*;
//!
//! #[crucible::quickcheck]
//! fn deposit_then_withdraw_is_identity(amount: i128) {
//!     let amount = amount.rem_euclid(1_000_000);
//!     let env = MockEnv::builder().with_contract::<Vault>().build();
//!     let vault = VaultClient::new(env.inner(), &env.contract_id::<Vault>());
//!     vault.deposit(&amount);
//!     assert_eq!(vault.withdraw(&amount), amount);
//! }
//! ```
//!
//! # Soroban type constraints
//!
//! Soroban's numeric types are not Rust's. A generator that yields arbitrary
//! `u64` values spends nearly all of its budget in a range no contract will
//! ever see, and `i128` values above the host's range cannot even cross the
//! contract boundary. [`Arbitrary`] implementations here are therefore biased
//! towards the values that actually find bugs: zero, one, the type's bounds,
//! and small magnitudes around them — see [`SorobanI128`], [`SorobanU32`] and
//! friends.
//!
//! # Determinism
//!
//! Generation is driven by a seeded [`Rng`], and a failing case reports the
//! seed that produced it. Re-running with `CRUCIBLE_QUICKCHECK_SEED` set to
//! that value reproduces the failure exactly.
//!
//! # Configuration
//!
//! | Environment variable | Meaning | Default |
//! | --- | --- | --- |
//! | `CRUCIBLE_QUICKCHECK_CASES` | Inputs to try per test | 256 |
//! | `CRUCIBLE_QUICKCHECK_SHRINK` | Maximum shrink steps | 1024 |
//! | `CRUCIBLE_QUICKCHECK_SEED` | Starting seed | random per run |
//!
//! The macro's own arguments — `#[crucible::quickcheck(cases = 32)]` — take
//! precedence over the environment.
//!
//! **Host-only:** this module depends on `std` and is intended exclusively for
//! `#[cfg(test)]` use on the host.

use std::fmt::Debug;

/// Default number of inputs generated per property.
pub const DEFAULT_CASES: u32 = 256;

/// Default cap on shrink steps before the smallest case found is reported.
pub const DEFAULT_SHRINK_ITERS: u32 = 1024;

/// A small deterministic PRNG (xorshift64*).
///
/// Chosen over pulling in a dependency: property generation needs speed and
/// reproducibility, not cryptographic quality.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Creates a generator from an explicit seed.
    ///
    /// A zero seed is remapped, since xorshift is degenerate at zero.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed },
        }
    }

    /// Returns the next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Returns the next raw 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Returns a value in `0..bound`, or `0` when `bound` is zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    /// Returns `true` with probability `1 / n` (always `false` when `n` is zero).
    pub fn one_in(&mut self, n: u64) -> bool {
        n > 0 && self.below(n) == 0
    }
}

/// A type that a property test can generate and shrink.
///
/// Implementations are provided for the primitives Soroban contracts accept,
/// for `Option`, `Vec`, `String` and tuples up to eight elements, and for the
/// Soroban-bounded newtypes in this module.
pub trait Arbitrary: Sized + Clone + Debug {
    /// Generates one value.
    ///
    /// `size` is a soft budget that bounds recursion and collection lengths, so
    /// generation always terminates.
    fn arbitrary(rng: &mut Rng, size: u32) -> Self;

    /// Returns progressively simpler candidates, simplest first.
    ///
    /// Shrinking stops when this yields nothing, so every chain must strictly
    /// decrease by some measure — never return `self`, or the search will not
    /// terminate.
    fn shrink(&self) -> Vec<Self> {
        Vec::new()
    }
}

/// Generates one of `choices`, weighted uniformly.
fn pick<T: Copy>(rng: &mut Rng, choices: &[T]) -> T {
    choices[rng.below(choices.len() as u64) as usize]
}

/// Implements [`Arbitrary`] for an unsigned primitive, biased toward the
/// boundary values that find off-by-one and overflow bugs.
macro_rules! impl_arbitrary_unsigned {
    ($($ty:ty),* $(,)?) => {$(
        impl Arbitrary for $ty {
            fn arbitrary(rng: &mut Rng, _size: u32) -> Self {
                // One case in four is an edge value; the rest are spread over
                // the full range so the interior is still covered.
                if rng.one_in(4) {
                    pick(rng, &[0, 1, 2, <$ty>::MAX, <$ty>::MAX - 1])
                } else {
                    rng.next_u64() as $ty
                }
            }

            fn shrink(&self) -> Vec<Self> {
                if *self == 0 {
                    return Vec::new();
                }
                // 0 and 1 first, so a boundary minimum is reached in one step.
                // Then a geometric ladder (half, three quarters, seven eighths,
                // ...) which binary-searches toward the true boundary instead
                // of decrementing towards it one value at a time.
                let mut candidates = vec![0, 1];
                let mut step = *self;
                while step > 1 {
                    step /= 2;
                    candidates.push(self - step);
                }
                candidates.push(self - 1);
                candidates.retain(|candidate| candidate < self);
                candidates.dedup();
                candidates
            }
        }
    )*};
}

/// Implements [`Arbitrary`] for a signed primitive.
macro_rules! impl_arbitrary_signed {
    ($($ty:ty),* $(,)?) => {$(
        impl Arbitrary for $ty {
            fn arbitrary(rng: &mut Rng, _size: u32) -> Self {
                if rng.one_in(4) {
                    pick(rng, &[0, 1, -1, <$ty>::MAX, <$ty>::MIN, <$ty>::MAX - 1])
                } else {
                    rng.next_u64() as $ty
                }
            }

            fn shrink(&self) -> Vec<Self> {
                if *self == 0 {
                    return Vec::new();
                }
                let mut candidates = vec![0, 1, -1];
                // A negative value shrinks toward its positive counterpart,
                // which reads better in a failure report. `MIN` has no
                // representable negation, so it is skipped.
                if *self != <$ty>::MIN && *self < 0 {
                    candidates.push(-self);
                }
                // A geometric ladder toward zero, so the boundary is found by
                // binary search rather than by stepping one value at a time.
                let mut step = self.unsigned_abs();
                while step > 1 {
                    step /= 2;
                    let delta = step as $ty;
                    candidates.push(if *self < 0 { self + delta } else { self - delta });
                }
                // Only keep strictly simpler values, measured by magnitude.
                let magnitude = self.unsigned_abs();
                candidates.retain(|candidate| candidate.unsigned_abs() < magnitude);
                candidates.dedup();
                candidates
            }
        }
    )*};
}

impl_arbitrary_unsigned!(u8, u16, u32, u64, u128, usize);
impl_arbitrary_signed!(i8, i16, i32, i64, i128, isize);

impl Arbitrary for bool {
    fn arbitrary(rng: &mut Rng, _size: u32) -> Self {
        rng.next_u32() & 1 == 1
    }

    fn shrink(&self) -> Vec<Self> {
        // `false` is the simpler value, so only `true` shrinks.
        if *self {
            vec![false]
        } else {
            Vec::new()
        }
    }
}

impl Arbitrary for char {
    fn arbitrary(rng: &mut Rng, _size: u32) -> Self {
        // ASCII only: Soroban strings and symbols are byte-oriented, and a
        // multi-byte scalar rarely tells you anything a byte would not.
        (rng.below(95) as u8 + 32) as char
    }

    fn shrink(&self) -> Vec<Self> {
        if *self == 'a' {
            Vec::new()
        } else {
            vec!['a']
        }
    }
}

impl Arbitrary for String {
    fn arbitrary(rng: &mut Rng, size: u32) -> Self {
        let len = rng.below(size.max(1) as u64);
        (0..len).map(|_| char::arbitrary(rng, size)).collect()
    }

    fn shrink(&self) -> Vec<Self> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut candidates = vec![String::new()];
        // Halve, then drop one character, mirroring the numeric strategy.
        candidates.push(self.chars().take(self.chars().count() / 2).collect());
        candidates.push(self.chars().take(self.chars().count() - 1).collect());
        candidates.retain(|candidate| candidate.len() < self.len());
        candidates.dedup();
        candidates
    }
}

impl<T: Arbitrary> Arbitrary for Option<T> {
    fn arbitrary(rng: &mut Rng, size: u32) -> Self {
        if rng.one_in(4) {
            None
        } else {
            Some(T::arbitrary(rng, size))
        }
    }

    fn shrink(&self) -> Vec<Self> {
        match self {
            None => Vec::new(),
            Some(value) => {
                let mut candidates = vec![None];
                candidates.extend(value.shrink().into_iter().map(Some));
                candidates
            }
        }
    }
}

impl<T: Arbitrary> Arbitrary for Vec<T> {
    fn arbitrary(rng: &mut Rng, size: u32) -> Self {
        let len = rng.below(size.max(1) as u64);
        (0..len).map(|_| T::arbitrary(rng, size)).collect()
    }

    fn shrink(&self) -> Vec<Self> {
        if self.is_empty() {
            return Vec::new();
        }
        // Length first — a shorter counterexample is almost always clearer
        // than a same-length one with simpler elements.
        let mut candidates = vec![Vec::new(), self[..self.len() / 2].to_vec()];
        for index in 0..self.len() {
            let mut without = self.clone();
            without.remove(index);
            candidates.push(without);
        }
        // Then element-wise simplification, one element at a time.
        for index in 0..self.len() {
            for simpler in self[index].shrink() {
                let mut candidate = self.clone();
                candidate[index] = simpler;
                candidates.push(candidate);
            }
        }
        candidates.retain(|candidate| candidate.len() <= self.len());
        candidates
    }
}

/// Implements [`Arbitrary`] for a tuple of the given arity.
///
/// Generation delegates to each element. Shrinking simplifies one position at
/// a time, holding the others fixed, so every candidate differs from the
/// original in exactly one place — which keeps the reported counterexample
/// readable.
macro_rules! impl_arbitrary_tuple {
    ($($name:ident => $index:tt),+ $(,)?) => {
        impl<$($name: Arbitrary),+> Arbitrary for ($($name,)+) {
            fn arbitrary(rng: &mut Rng, size: u32) -> Self {
                ($($name::arbitrary(rng, size),)+)
            }

            fn shrink(&self) -> Vec<Self> {
                let mut candidates = Vec::new();
                $(
                    for simpler in self.$index.shrink() {
                        let mut candidate = self.clone();
                        candidate.$index = simpler;
                        candidates.push(candidate);
                    }
                )+
                candidates
            }
        }
    };
}

impl_arbitrary_tuple!(A => 0);
impl_arbitrary_tuple!(A => 0, B => 1);
impl_arbitrary_tuple!(A => 0, B => 1, C => 2);
impl_arbitrary_tuple!(A => 0, B => 1, C => 2, D => 3);
impl_arbitrary_tuple!(A => 0, B => 1, C => 2, D => 3, E => 4);
impl_arbitrary_tuple!(A => 0, B => 1, C => 2, D => 3, E => 4, F => 5);
impl_arbitrary_tuple!(A => 0, B => 1, C => 2, D => 3, E => 4, F => 5, G => 6);
impl_arbitrary_tuple!(A => 0, B => 1, C => 2, D => 3, E => 4, F => 5, G => 6, H => 7);

/// Declares a newtype whose generator is restricted to a Soroban-legal range.
macro_rules! soroban_bounded {
    (
        $(#[$meta:meta])*
        $name:ident($inner:ty), min = $min:expr, max = $max:expr, edges = [$($edge:expr),* $(,)?]
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $inner);

        impl $name {
            /// The smallest value this generator produces.
            pub const MIN: $inner = $min;
            /// The largest value this generator produces.
            pub const MAX: $inner = $max;

            /// Returns the wrapped value.
            pub fn get(self) -> $inner {
                self.0
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl Arbitrary for $name {
            fn arbitrary(rng: &mut Rng, _size: u32) -> Self {
                if rng.one_in(3) {
                    return Self(pick(rng, &[$($edge),*]));
                }
                // `MAX - MIN` can exceed the range of the type itself, so the
                // span is computed in the widest unsigned type available.
                let span = ($max as i128).wrapping_sub($min as i128).unsigned_abs();
                let offset = if span == 0 {
                    0
                } else {
                    // Two draws, because `span` may exceed 64 bits for i128.
                    let high = (rng.next_u64() as u128) << 64;
                    (high | rng.next_u64() as u128) % (span.saturating_add(1))
                };
                Self(($min as i128).saturating_add(offset as i128) as $inner)
            }

            fn shrink(&self) -> Vec<Self> {
                // The bounds check is trivially true for newtypes that span
                // their whole underlying type, and meaningful for the ones that
                // do not; the lint is silenced rather than the check dropped.
                #[allow(unused_comparisons)]
                self.0
                    .shrink()
                    .into_iter()
                    .filter(|candidate| (*candidate >= $min) && (*candidate <= $max))
                    .map(Self)
                    .collect()
            }
        }
    };
}

soroban_bounded! {
    /// An `i128` restricted to the range Soroban contracts actually use for
    /// token amounts: non-negative and within the host's signed 128-bit range.
    ///
    /// Generated values cluster around `0`, `1` and the maximum, which is where
    /// overflow and underflow bugs live.
    SorobanAmount(i128), min = 0, max = i128::MAX, edges = [0, 1, 2, i128::MAX, i128::MAX - 1]
}

soroban_bounded! {
    /// A full-range `i128`, including negatives.
    ///
    /// Use this to check that a contract rejects negative amounts rather than
    /// wrapping or panicking on them.
    SorobanI128(i128), min = i128::MIN, max = i128::MAX, edges = [0, 1, -1, i128::MAX, i128::MIN]
}

soroban_bounded! {
    /// A `u32` in the range Soroban uses for ledger sequence numbers.
    SorobanU32(u32), min = 0, max = u32::MAX, edges = [0, 1, u32::MAX, u32::MAX - 1]
}

soroban_bounded! {
    /// A `u64` in the range Soroban uses for ledger timestamps.
    SorobanTimestamp(u64), min = 0, max = u64::MAX, edges = [0, 1, u64::MAX, u64::MAX - 1]
}

/// How a property run should be configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Number of inputs to generate.
    pub cases: u32,
    /// Maximum shrink steps before reporting the smallest case found.
    pub shrink_iters: u32,
    /// Seed for the generator, or `None` to pick one per run.
    pub seed: Option<u64>,
    /// Soft budget bounding collection lengths and recursion.
    pub size: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cases: DEFAULT_CASES,
            shrink_iters: DEFAULT_SHRINK_ITERS,
            seed: None,
            size: 32,
        }
    }
}

impl Config {
    /// Applies the `CRUCIBLE_QUICKCHECK_*` environment overrides.
    ///
    /// Values set explicitly by the macro are kept; only the fields the caller
    /// left at their default are overridden, so
    /// `#[crucible::quickcheck(cases = 8)]` is not silently widened by a
    /// developer's shell.
    pub fn from_env(mut self) -> Self {
        fn read<T: std::str::FromStr>(name: &str) -> Option<T> {
            std::env::var(name).ok()?.trim().parse().ok()
        }
        if self.cases == DEFAULT_CASES {
            if let Some(cases) = read("CRUCIBLE_QUICKCHECK_CASES") {
                self.cases = cases;
            }
        }
        if self.shrink_iters == DEFAULT_SHRINK_ITERS {
            if let Some(iters) = read("CRUCIBLE_QUICKCHECK_SHRINK") {
                self.shrink_iters = iters;
            }
        }
        if self.seed.is_none() {
            self.seed = read("CRUCIBLE_QUICKCHECK_SEED");
        }
        self
    }

    /// Returns the seed to run with, choosing one if none was fixed.
    fn resolved_seed(&self) -> u64 {
        self.seed.unwrap_or_else(|| {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x5EED)
        })
    }
}

/// Runs `property` against generated inputs, shrinking any failure.
///
/// This is what [`#[crucible::quickcheck]`][macro@crate::quickcheck] expands
/// to. Call it directly only when building a bespoke harness.
///
/// # Panics
///
/// Panics when a generated input makes `property` panic, reporting the
/// shrunken input, the seed, and the original panic message.
#[track_caller]
pub fn check<T, F>(name: &str, config: Config, mut property: F)
where
    T: Arbitrary,
    F: FnMut(T),
{
    let config = config.from_env();
    let seed = config.resolved_seed();
    let mut rng = Rng::new(seed);

    for case in 0..config.cases {
        let input = T::arbitrary(&mut rng, config.size);
        if let Some(failure) = run_once(&mut property, input.clone()) {
            let (minimal, minimal_failure, steps) =
                shrink(&mut property, input, failure, config.shrink_iters);
            panic!(
                "{name} failed on case {case} of {cases}.\n\
                 \n\
                 Minimal failing input : {minimal:?}\n\
                 Panic                 : {message}\n\
                 Shrink steps          : {steps}\n\
                 Seed                  : {seed}\n\
                 \n\
                 Re-run this exact case with CRUCIBLE_QUICKCHECK_SEED={seed}.",
                name = name,
                case = case + 1,
                cases = config.cases,
                minimal = minimal,
                message = minimal_failure,
                steps = steps,
                seed = seed,
            );
        }
    }
}

/// Runs `property` once, returning the panic message if it failed.
fn run_once<T, F>(property: &mut F, input: T) -> Option<String>
where
    T: Arbitrary,
    F: FnMut(T),
{
    // Property bodies are ordinary test code and panic on assertion failure;
    // catching lets the harness shrink instead of aborting the whole test.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| property(input)));
    std::panic::set_hook(previous);

    match outcome {
        Ok(()) => None,
        Err(payload) => Some(panic_message(payload.as_ref())),
    }
}

/// Repeatedly replaces the failing input with a simpler one that still fails.
///
/// Returns the smallest input found, its panic message, and how many
/// replacements were made. The search is greedy: at each step the first
/// candidate that still fails is taken, which keeps the walk linear in the
/// number of shrink steps rather than exploring the whole candidate tree.
fn shrink<T, F>(
    property: &mut F,
    mut input: T,
    mut failure: String,
    max_steps: u32,
) -> (T, String, u32)
where
    T: Arbitrary,
    F: FnMut(T),
{
    let mut steps = 0;
    'outer: while steps < max_steps {
        for candidate in input.shrink() {
            if let Some(candidate_failure) = run_once(property, candidate.clone()) {
                input = candidate;
                failure = candidate_failure;
                steps += 1;
                continue 'outer;
            }
        }
        // No simpler candidate reproduced the failure, so this is a local
        // minimum and the search is done.
        break;
    }
    (input, failure, steps)
}

/// Renders a `catch_unwind` payload as a message.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a property, returning the panic message instead of failing.
    fn check_reporting<T, F>(config: Config, property: F) -> Option<String>
    where
        T: Arbitrary,
        F: FnMut(T),
    {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check::<T, F>("property", config, property)
        }));
        std::panic::set_hook(previous);
        outcome.err().map(|payload| panic_message(payload.as_ref()))
    }

    fn fixed_seed() -> Config {
        Config {
            seed: Some(0xC0FF_EE00),
            ..Config::default()
        }
    }

    #[test]
    fn the_rng_is_deterministic_for_a_seed() {
        let a: Vec<u64> = (0..8).map(|_| Rng::new(7).next_u64()).collect();
        let b: Vec<u64> = (0..8).map(|_| Rng::new(7).next_u64()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_diverge() {
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn a_holding_property_passes() {
        check::<i32, _>("addition commutes", fixed_seed(), |x| {
            assert_eq!(x.wrapping_add(0), x);
        });
    }

    #[test]
    fn a_failing_property_reports_the_seed_for_reproduction() {
        let report = check_reporting::<u32, _>(fixed_seed(), |x| {
            assert!(x < 10, "value too large");
        })
        .expect("the property must fail");

        assert!(report.contains("CRUCIBLE_QUICKCHECK_SEED="), "{report}");
        assert!(report.contains("Minimal failing input"), "{report}");
    }

    #[test]
    fn shrinking_reduces_an_integer_to_the_boundary() {
        // Every value from 10 upwards fails, so the minimal case is exactly 10.
        let report = check_reporting::<u32, _>(fixed_seed(), |x| {
            assert!(x < 10, "too large");
        })
        .expect("the property must fail");

        assert!(
            report.contains("Minimal failing input : 10"),
            "expected the shrinker to reach 10, got:\n{report}"
        );
    }

    #[test]
    fn shrinking_reduces_a_vector_to_the_shortest_failing_case() {
        // Any vector of two or more elements fails, so the minimum is length 2.
        let report = check_reporting::<Vec<u8>, _>(
            Config {
                cases: 512,
                ..fixed_seed()
            },
            |values| {
                assert!(values.len() < 2, "too long");
            },
        )
        .expect("the property must fail");

        assert!(
            report.contains("Minimal failing input : [0, 0]"),
            "expected a two-element minimum, got:\n{report}"
        );
    }

    #[test]
    fn shrinking_a_tuple_simplifies_every_position() {
        let report = check_reporting::<(u32, u32), _>(
            Config {
                cases: 512,
                ..fixed_seed()
            },
            |(a, b)| {
                assert!(a < 5 || b < 5, "both too large");
            },
        )
        .expect("the property must fail");

        assert!(
            report.contains("Minimal failing input : (5, 5)"),
            "expected both positions to shrink to 5, got:\n{report}"
        );
    }

    #[test]
    fn a_fixed_seed_reproduces_the_same_minimal_case() {
        let run = || {
            check_reporting::<u64, _>(fixed_seed(), |x| {
                assert!(x < 1_000, "too large");
            })
            .expect("the property must fail")
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn shrink_chains_always_terminate() {
        // A shrink candidate that is not strictly simpler would loop forever.
        for value in [0_u32, 1, 2, 7, u32::MAX] {
            for candidate in value.shrink() {
                assert!(candidate < value, "{candidate} is not simpler than {value}");
            }
        }
        for value in [0_i64, -1, 1, i64::MIN, i64::MAX] {
            for candidate in value.shrink() {
                assert!(
                    candidate.unsigned_abs() < value.unsigned_abs(),
                    "{candidate} is not simpler than {value}"
                );
            }
        }
    }

    #[test]
    fn zero_and_false_are_already_minimal() {
        assert!(0_u32.shrink().is_empty());
        assert!(0_i128.shrink().is_empty());
        assert!(false.shrink().is_empty());
        assert!(String::new().shrink().is_empty());
        assert!(Vec::<u8>::new().shrink().is_empty());
        assert!(None::<u32>.shrink().is_empty());
    }

    #[test]
    fn soroban_amounts_are_never_negative() {
        let mut rng = Rng::new(99);
        for _ in 0..1_000 {
            assert!(SorobanAmount::arbitrary(&mut rng, 32).get() >= 0);
        }
    }

    #[test]
    fn soroban_newtypes_shrink_within_their_bounds() {
        // `SorobanAmount` is the non-negative half of `i128`, so its shrink
        // chain must never cross into the negatives even though the underlying
        // `i128` shrinker offers `-1` as a candidate.
        for candidate in SorobanAmount(i128::MAX).shrink() {
            assert!(
                candidate.get() >= SorobanAmount::MIN,
                "{candidate:?} escaped the lower bound"
            );
        }
        assert!(SorobanAmount(i128::MAX)
            .shrink()
            .iter()
            .all(|candidate| candidate.get() >= 0));
    }

    #[test]
    fn generators_reach_their_boundary_values() {
        let mut rng = Rng::new(5);
        let mut saw_zero = false;
        let mut saw_max = false;
        for _ in 0..10_000 {
            match SorobanAmount::arbitrary(&mut rng, 32).get() {
                0 => saw_zero = true,
                v if v == i128::MAX => saw_max = true,
                _ => {}
            }
        }
        assert!(saw_zero, "the generator must produce 0");
        assert!(saw_max, "the generator must produce i128::MAX");
    }

    #[test]
    fn explicit_config_is_not_widened_by_the_environment() {
        // `cases` was set explicitly, so `from_env` must leave it alone even
        // though the default would have been overridden.
        let config = Config {
            cases: 8,
            ..Config::default()
        };
        assert_eq!(config.from_env().cases, 8);
    }
}
