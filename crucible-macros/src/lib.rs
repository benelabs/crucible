//! Proc-macro support for the [`crucible`](https://docs.rs/crucible) Soroban testing framework.
//!
//! This crate provides the [`#[fixture]`][macro@fixture] attribute macro used to reduce
//! boilerplate in Soroban contract test setups. It is re-exported from the main `crucible`
//! crate under the `derive` feature (enabled by default), so you normally import it as:
//!
//! ```rust,ignore
//! use crucible::fixture;
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Error};

/// Marks a struct as a reusable test fixture.
///
/// This attribute macro does two things:
///
/// 1. **Auto-derives [`Debug`]** — adds `#[derive(Debug)]` to the struct if it is not
///    already present, so fixture values can be printed in test failure output.
///
/// 2. **Injects `reset(&mut self)`** — generates a method that calls `Self::setup()` and
///    assigns the result to `*self`, allowing a fixture to be cheaply reset to its initial
///    state at any point inside a test.
///
/// # Requirements
///
/// The annotated struct **must** have a user-supplied `impl` block containing:
///
/// ```rust,ignore
/// pub fn setup() -> Self { /* ... */ }
/// ```
///
/// If `setup()` is absent the code will not compile; the compiler will emit an error
/// indicating that no associated function `setup` was found on the type.
///
/// # Generated code
///
/// Given:
///
/// ```rust,ignore
/// #[fixture]
/// pub struct CounterFixture {
///     pub count: u32,
/// }
/// ```
///
/// The macro expands to (approximately):
///
/// ```rust,ignore
/// #[derive(Debug)]
/// pub struct CounterFixture {
///     pub count: u32,
/// }
///
/// impl CounterFixture {
///     /// Resets the fixture to its initial state by calling [`Self::setup()`].
///     pub fn reset(&mut self) {
///         *self = Self::setup();
///     }
/// }
/// ```
///
/// # Examples
///
/// ```rust
/// use crucible_macros::fixture;
///
/// #[fixture]
/// pub struct CounterFixture {
///     pub count: u32,
/// }
///
/// impl CounterFixture {
///     pub fn setup() -> Self {
///         Self { count: 0 }
///     }
/// }
///
/// let mut f = CounterFixture::setup();
/// assert_eq!(f.count, 0);
///
/// f.count = 42;
/// f.reset();
/// assert_eq!(f.count, 0); // reset() calls setup() and replaces self
/// ```
#[proc_macro_attribute]
pub fn fixture(args: TokenStream, input: TokenStream) -> TokenStream {
    // #[fixture] takes no arguments.
    let args2 = proc_macro2::TokenStream::from(args);
    if !args2.is_empty() {
        return Error::new_spanned(args2, "#[fixture] does not take arguments")
            .to_compile_error()
            .into();
    }

    let mut ast = parse_macro_input!(input as DeriveInput);

    // Only structs are supported.
    if !matches!(ast.data, Data::Struct(_)) {
        return Error::new_spanned(&ast.ident, "#[fixture] can only be applied to structs")
            .to_compile_error()
            .into();
    }

    let ident = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // Add #[derive(Debug)] if the user has not already derived it.
    if !has_derive(&ast.attrs, "Debug") {
        let debug_attr: syn::Attribute = syn::parse_quote!(#[derive(Debug)]);
        ast.attrs.push(debug_attr);
    }

    let expanded = quote! {
        #ast

        impl #impl_generics #ident #ty_generics #where_clause {
            /// Resets the fixture to its initial state by calling [`Self::setup()`].
            ///
            /// This is a convenience shorthand for `*self = Self::setup()`.  Use it to
            /// restore a clean environment between logical sections of a single test.
            ///
            /// # Compile error
            ///
            /// If you see a compiler error pointing here, add a `pub fn setup() -> Self`
            /// associated function to the struct's `impl` block.
            pub fn reset(&mut self) {
                *self = Self::setup();
            }
        }
    };

    expanded.into()
}

/// Returns `true` if any `#[derive(...)]` attribute in `attrs` lists the given `name`.
fn has_derive(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }
        attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
        )
        .map(|paths| paths.iter().any(|p| p.is_ident(name)))
        .unwrap_or(false)
    })
}
