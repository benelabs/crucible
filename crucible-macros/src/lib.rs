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
                                // `#[contract_client(contract = T)]` parses as a
                                // name-value pair whose value is an expression, so
                                // the type is reached through `Expr::Path`. The bare
                                // `#[contract_client(contract)]` form is also
                                // tolerated and handled by the `Meta::Path` arm.
                                match &m {
                                    Meta::NameValue(nv) if nv.path.is_ident("contract") => {
                                        if let syn::Expr::Path(expr) = &nv.value {
                                            contract_ty = Some(expr.path.clone());
                                        }
                                    }
                                    Meta::Path(path) if !path.is_ident("contract") => {
                                        contract_ty = Some(path.clone());
                                    }
                                    _ => {}
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

/// Turns a function into a property-based fuzz test.
///
/// The annotated function's parameters are treated as *generated* inputs: the
/// macro emits a `#[test]` that runs the body against many random values and,
/// when the body panics, shrinks the input to a minimal reproducing case before
/// reporting it.
///
/// Every parameter type must implement [`crucible::quickcheck::Arbitrary`],
/// which is provided for the integer primitives, `bool`, `char`, `String`,
/// `Option<T>`, `Vec<T>`, tuples, and the Soroban-bounded newtypes such as
/// `SorobanAmount`.
///
/// # Arguments
///
/// * `cases = N` — number of inputs to generate (default 256).
/// * `shrink = N` — maximum shrink steps (default 1024).
/// * `seed = N` — fix the seed, making the run fully reproducible.
/// * `size = N` — soft budget bounding generated collection lengths (default 32).
///
/// Unset arguments fall back to the `CRUCIBLE_QUICKCHECK_CASES`,
/// `CRUCIBLE_QUICKCHECK_SHRINK` and `CRUCIBLE_QUICKCHECK_SEED` environment
/// variables, then to the defaults above.
///
/// # Example
///
/// ```rust,ignore
/// use crucible::prelude::*;
///
/// #[crucible::quickcheck]
/// fn minting_never_decreases_the_balance(amount: SorobanAmount) {
///     let amount = amount.get() % 1_000_000;
///     let env = MockEnv::builder().build();
///     let token = MockToken::xlm(&env);
///     let alice = AccountBuilder::new(&env).name("alice").build();
///
///     let before = token.balance(&alice.address());
///     token.mint(&alice.address(), amount);
///     assert!(token.balance(&alice.address()) >= before);
/// }
///
/// #[crucible::quickcheck(cases = 32, seed = 42)]
/// fn addition_is_commutative(a: i64, b: i64) {
///     assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
/// }
/// ```
///
/// # Generated code
///
/// The body is moved into a closure that destructures the generated tuple, so
/// parameter names and patterns work exactly as written:
///
/// ```rust,ignore
/// #[test]
/// fn addition_is_commutative() {
///     crucible::quickcheck::check::<(i64, i64), _>(
///         "addition_is_commutative",
///         crucible::quickcheck::Config { cases: 32, seed: Some(42), ..Default::default() },
///         |(a, b)| { /* original body */ },
///     );
/// }
/// ```
#[proc_macro_attribute]
pub fn quickcheck(args: TokenStream, input: TokenStream) -> TokenStream {
    let config = match QuickcheckArgs::parse(args.into()) {
        Ok(config) => config,
        Err(err) => return err.to_compile_error().into(),
    };

    let func = parse_macro_input!(input as syn::ItemFn);
    match expand_quickcheck(config, func) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// The `cases` / `shrink` / `seed` / `size` arguments of `#[quickcheck]`.
#[derive(Default)]
struct QuickcheckArgs {
    cases: Option<syn::Expr>,
    shrink: Option<syn::Expr>,
    seed: Option<syn::Expr>,
    size: Option<syn::Expr>,
}

impl QuickcheckArgs {
    /// Parses a comma-separated list of `name = value` pairs.
    fn parse(tokens: proc_macro2::TokenStream) -> Result<Self, Error> {
        let mut parsed = Self::default();
        if tokens.is_empty() {
            return Ok(parsed);
        }

        let metas = syn::parse::Parser::parse2(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
            tokens,
        )?;

        for meta in metas {
            let slot = if meta.path.is_ident("cases") {
                &mut parsed.cases
            } else if meta.path.is_ident("shrink") {
                &mut parsed.shrink
            } else if meta.path.is_ident("seed") {
                &mut parsed.seed
            } else if meta.path.is_ident("size") {
                &mut parsed.size
            } else {
                return Err(Error::new_spanned(
                    &meta.path,
                    "unknown #[quickcheck] argument; expected one of `cases`, `shrink`, `seed`, `size`",
                ));
            };

            if slot.is_some() {
                return Err(Error::new_spanned(
                    &meta.path,
                    "duplicate #[quickcheck] argument",
                ));
            }
            *slot = Some(meta.value);
        }

        Ok(parsed)
    }
}

/// Builds the `#[test]` function that drives the property.
fn expand_quickcheck(
    args: QuickcheckArgs,
    func: syn::ItemFn,
) -> Result<proc_macro2::TokenStream, Error> {
    let syn::ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = func;

    if let Some(asyncness) = sig.asyncness {
        return Err(Error::new_spanned(
            asyncness,
            "#[quickcheck] does not support async functions",
        ));
    }
    if !matches!(sig.output, syn::ReturnType::Default) {
        return Err(Error::new_spanned(
            &sig.output,
            "#[quickcheck] properties must return `()`; assert inside the body instead",
        ));
    }
    if sig.inputs.is_empty() {
        return Err(Error::new_spanned(
            &sig.ident,
            "#[quickcheck] requires at least one parameter to generate; \
             a function with no inputs is an ordinary #[test]",
        ));
    }

    // Split the parameter list into the patterns to destructure and the types
    // to generate. A generated tuple is destructured back into the original
    // patterns, so the body needs no rewriting.
    let mut patterns = Vec::new();
    let mut types = Vec::new();
    for arg in &sig.inputs {
        match arg {
            syn::FnArg::Typed(pat_type) => {
                patterns.push(pat_type.pat.clone());
                types.push(pat_type.ty.clone());
            }
            syn::FnArg::Receiver(receiver) => {
                return Err(Error::new_spanned(
                    receiver,
                    "#[quickcheck] cannot be applied to methods taking `self`",
                ));
            }
        }
    }

    let name = &sig.ident;
    let name_literal = name.to_string();
    let generics = &sig.generics;
    let where_clause = &sig.generics.where_clause;

    let field = |value: &Option<syn::Expr>, name: &str| -> proc_macro2::TokenStream {
        let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
        match value {
            Some(expr) => quote! { #ident: #expr, },
            None => quote! {},
        }
    };
    let cases = field(&args.cases, "cases");
    let shrink = field(&args.shrink, "shrink_iters");
    let size = field(&args.size, "size");
    // `seed` is an `Option<u64>` in the config, so the literal has to be wrapped.
    let seed = match &args.seed {
        Some(expr) => quote! { seed: ::core::option::Option::Some(#expr), },
        None => quote! {},
    };

    Ok(quote! {
        #(#attrs)*
        #[test]
        #vis fn #name #generics () #where_clause {
            ::crucible::quickcheck::check::<( #(#types,)* ), _>(
                #name_literal,
                ::crucible::quickcheck::Config {
                    #cases
                    #shrink
                    #seed
                    #size
                    ..::core::default::Default::default()
                },
                |( #(#patterns,)* )| #block,
            );
        }
    })
}
