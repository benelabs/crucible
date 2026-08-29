//! Proc-macro support for the [`crucible`](https://docs.rs/crucible) Soroban testing framework.
//!
//! This crate provides the [`#[fixture]`][macro@fixture] attribute macro and the
//! [`#[derive(Fixture)]`][macro@Fixture] derive macro used to reduce boilerplate
//! in Soroban contract test setups. They are re-exported from the main `crucible`
//! crate under the `derive` feature (enabled by default), so you normally import
//! them as:
//!
//! ```rust,ignore
//! use crucible::fixture;
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Error, Fields, Meta};

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

/// Derive macro for generating typed contract client wrappers in test fixtures.
///
/// This derive macro can be applied to a struct to automatically generate a
/// `setup()` method that initializes the fixture fields. It supports
/// auto-wiring contract clients via the `#[contract_client(contract = T)]`
/// field attribute.
///
/// # Requirements
///
/// The struct must have an `env: MockEnv` field. The macro will auto-derive
/// `Debug` if not already present.
///
/// # Example
///
/// ```rust,ignore
/// use crucible_macros::Fixture;
/// use crucible::prelude::*;
///
/// #[derive(Fixture)]
/// struct DexFixture {
///     env: MockEnv,
///     #[contract_client(contract = AmmPool)]
///     pool_client: AmmPoolClient,
/// }
/// ```
///
/// This expands to a `setup()` method that creates the `MockEnv` and wires
/// the `pool_client` using `env.contract_id::<AmmPool>()`.
#[proc_macro_derive(Fixture, attributes(contract_client))]
pub fn fixture_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    // Cloned rather than borrowed: `input` is moved into `ast` below, and
    // `name` is still needed after that move.
    let name = input.ident.clone();

    let mut has_debug = false;
    for attr in &input.attrs {
        if attr.path().is_ident("derive") {
            let _ = attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            ).map(|paths| {
                if paths.iter().any(|p| p.is_ident("Debug")) {
                    has_debug = true;
                }
            });
        }
    }

    let mut ast = input;
    if !has_debug {
        let debug_attr: syn::Attribute = syn::parse_quote!(#[derive(Debug)]);
        ast.attrs.push(debug_attr);
    }

    let mut contract_types = Vec::new();
    let mut field_inits = Vec::new();
    let mut has_env = false;

    if let Data::Struct(data) = &ast.data {
        if let Fields::Named(fields) = &data.fields {
            for field in &fields.named {
                let field_name = field.ident.as_ref().unwrap();
                let field_ty = &field.ty;

                if field_name == "env" {
                    has_env = true;
                    continue;
                }

                let mut is_contract_client = false;
                let mut contract_ty = None;

                for attr in &field.attrs {
                    if attr.path().is_ident("contract_client") {
                        is_contract_client = true;
                        let meta = attr.parse_args_with(
                            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                        );
                        if let Ok(metas) = meta {
                            for m in metas {
                                if m.path().is_ident("contract") {
                                    if let Meta::Path(path) = m {
                                        contract_ty = Some(path);
                                // `#[contract_client(contract = T)]` parses as
                                // a name-value pair whose value is an
                                // expression, so the type is reached through
                                // `Expr::Path` — `Meta` itself has no `value`.
                                if let Meta::NameValue(nv) = &m {
                                    if nv.path.is_ident("contract") {
                                        if let syn::Expr::Path(expr) = &nv.value {
                                            contract_ty = Some(expr.path.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if is_contract_client {
                    if let Some(ct) = contract_ty {
                        contract_types.push(ct.clone());
                        field_inits.push(quote! {
                            #field_name: <#field_ty>::new(env.inner(), &env.contract_id::<#ct>()),
                        });
                    }
                } else {
                    field_inits.push(quote! {
                        #field_name: Default::default(),
                    });
                }
            }
        }
    }

    if !has_env {
        return syn::Error::new_spanned(name, "#[derive(Fixture)] requires an `env: MockEnv` field")
            .to_compile_error()
            .into();
    }

    let with_contracts = contract_types.iter().map(|ty| quote! { .with_contract::<#ty>() });
    let env_init = quote! { MockEnv::builder() #(#with_contracts)* .build() };

    let expanded = quote! {
        #ast

        impl #name {
            /// Creates a new instance with all fields initialized.
            ///
            /// This method is generated by the `#[derive(Fixture)]` macro. It creates
            /// a fresh `MockEnv` and wires all `#[contract_client]` fields. Other
            /// fields (besides `env`) are set to their `Default` value.
            pub fn setup() -> Self {
                let env = #env_init;
                Self {
                    env,
                    #(#field_inits)*
                }
            }
        }
    };

    expanded.into()
}
