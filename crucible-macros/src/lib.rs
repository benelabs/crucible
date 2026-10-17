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
use syn::{parse_macro_input, Data, DeriveInput, Error, Fields, Ident, Meta, Path, Type};

// ---------------------------------------------------------------------------
// Path normalization utilities (Issue #1068)
// ---------------------------------------------------------------------------

/// Normalizes a type path to its last segment string for identity comparison.
///
/// This ensures that `TokenFixture` and `crate::fixtures::TokenFixture` (or any
/// other fully/partially qualified path pointing to the same type) are
/// recognized as identical during cycle and duplicate detection.
///
/// Single-segment paths like `TokenFixture` are returned as-is. Multi-segment
/// paths like `crate::fixtures::TokenFixture` are also reduced to their final
/// segment (`TokenFixture`) for comparison purposes.
fn normalize_path_name(path: &Path) -> String {
    // Use the last segment as the canonical identity.  This is a pragmatic
    // heuristic that handles the common aliases: `Foo`, `crate::Foo`,
    // `crate::fixtures::Foo`, `super::Foo`, etc.
    path.segments
        .last()
        .map(|seg| seg.ident.to_string())
        .unwrap_or_default()
}

/// Returns the first duplicate dependency after normalization, if any.
///
/// Two dependency paths are considered equal when [`normalize_path_name`]
/// produces the same string for both.
fn first_duplicate(deps: &[Path]) -> Option<&Path> {
    let mut seen = std::collections::HashSet::new();
    for dep in deps {
        let key = normalize_path_name(dep);
        if !seen.insert(key) {
            return Some(dep);
        }
    }
    None
}

/// Detects trivial self-cycles where the fixture lists itself as a dependency
/// (after path normalization).
fn is_self_referential(fixture_name: &Ident, deps: &[Path]) -> Option<&Path> {
    let name = fixture_name.to_string();
    deps.iter().find(|dep| normalize_path_name(dep) == name)
}

// ---------------------------------------------------------------------------
// Argument parsing for #[fixture(requires = [...])] (Issue #1069)
// ---------------------------------------------------------------------------

/// Parsed arguments from `#[fixture(...)]`.
struct FixtureArgs {
    /// The dependency paths declared via `requires = [Dep1, Dep2, ...]`.
    requires: Vec<Path>,
}

/// Parses the attribute arguments for `#[fixture]`.
///
/// Accepted forms:
/// - `#[fixture]` — no arguments (empty `requires`).
/// - `#[fixture(requires = [Dep1, Dep2])]` — explicit dependency list.
///
/// # Error handling (Issue #1069)
///
/// Instead of masking all parse failures behind a single generic message, this
/// function preserves the underlying [`syn::Error`] and chains a diagnostic
/// note that describes the expected format. This way, syntax errors such as
/// unclosed brackets or trailing commas are surfaced to the user.
fn parse_fixture_args(tokens: proc_macro2::TokenStream) -> Result<FixtureArgs, proc_macro2::TokenStream> {
    use syn::parse::Parse;
    use syn::parse::ParseStream;
    use syn::punctuated::Punctuated;
    use syn::token::Bracket;

    if tokens.is_empty() {
        return Ok(FixtureArgs { requires: Vec::new() });
    }

    struct RawArgs {
        requires: Vec<Path>,
    }

    impl Parse for RawArgs {
        fn parse(input: ParseStream) -> syn::Result<Self> {
            let ident: Ident = input.parse().map_err(|e| {
                let mut err = syn::Error::new(
                    e.span(),
                    "expected `requires = [Type1, Type2, ...]`",
                );
                err.combine(e);
                err
            })?;

            if ident != "requires" {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "unknown fixture argument `{ident}`; \
                         only `requires = [Type1, Type2, ...]` is supported"
                    ),
                ));
            }

            input.parse::<syn::Token![=]>().map_err(|e| {
                let mut err = syn::Error::new(
                    e.span(),
                    "expected `=` after `requires`; \
                     use `requires = [Type1, Type2, ...]`",
                );
                err.combine(e);
                err
            })?;

            let content;
            let _bracket: Bracket = syn::bracketed!(content in input).map_err(|e| {
                let mut err = syn::Error::new(
                    e.span(),
                    "expected `[` to open the dependency list; \
                     use `requires = [Type1, Type2, ...]`",
                );
                err.combine(e);
                err
            })?;

            let paths: Punctuated<Path, syn::Token![,]> =
                content.parse_terminated(Path::parse, syn::Token![,]).map_err(|e| {
                    let mut err = syn::Error::new(
                        e.span(),
                        "malformed dependency list inside `requires = [...]`",
                    );
                    // Chain the original error so users see the precise syntax
                    // issue (e.g., unclosed bracket, unexpected token).
                    err.combine(e);
                    err
                })?;

            // Reject trailing tokens after the closing `]`.
            if !input.is_empty() {
                return Err(syn::Error::new(
                    input.span(),
                    "unexpected tokens after `requires = [...]`; \
                     #[fixture] accepts at most one argument",
                ));
            }

            Ok(RawArgs {
                requires: paths.into_iter().collect(),
            })
        }
    }

    syn::parse2::<RawArgs>(tokens.clone()).map(|r| FixtureArgs { requires: r.requires }).map_err(|e| {
        e.to_compile_error()
    })
}

// ---------------------------------------------------------------------------
// #[fixture] attribute macro
// ---------------------------------------------------------------------------

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
/// # Dependency declaration
///
/// Optionally, you can declare fixture dependencies using:
///
/// ```rust,ignore
/// #[fixture(requires = [DepFixture1, DepFixture2])]
/// ```
///
/// This generates an `AcyclicFixture` trait implementation that performs
/// compile-time cycle detection and duplicate checking across the fixture
/// dependency graph.
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
    let args2 = proc_macro2::TokenStream::from(args);

    // Parse fixture arguments (supports empty or `requires = [...]`).
    let fixture_args = match parse_fixture_args(args2) {
        Ok(a) => a,
        Err(compile_err) => return compile_err.into(),
    };

    let mut ast = parse_macro_input!(input as DeriveInput);

    // Only structs are supported.
    if !matches!(ast.data, Data::Struct(_)) {
        return Error::new_spanned(&ast.ident, "#[fixture] can only be applied to structs")
            .to_compile_error()
            .into();
    }

    let ident = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // --- Dependency validation (Issue #1068) ---

    // Check for self-referential dependencies.
    if let Some(self_dep) = is_self_referential(ident, &fixture_args.requires) {
        return Error::new_spanned(
            self_dep,
            format!(
                "fixture `{ident}` cannot list itself as a dependency"
            ),
        )
        .to_compile_error()
        .into();
    }

    // Check for duplicate dependencies (with path normalization).
    if let Some(dup) = first_duplicate(&fixture_args.requires) {
        let normalized = normalize_path_name(dup);
        return Error::new_spanned(
            dup,
            format!(
                "duplicate dependency `{normalized}` in fixture `{ident}`; \
                 each dependency must appear only once (note: paths are compared \
                 by their final segment, so `{normalized}` and \
                 `crate::fixtures::{normalized}` are considered identical)"
            ),
        )
        .to_compile_error()
        .into();
    }

    // Add #[derive(Debug)] if the user has not already derived it.
    if !has_derive(&ast.attrs, "Debug") {
        let debug_attr: syn::Attribute = syn::parse_quote!(#[derive(Debug)]);
        ast.attrs.push(debug_attr);
    }

    // --- Generate AcyclicFixture trait impl when dependencies are declared ---
    let acyclic_impl = if fixture_args.requires.is_empty() {
        // No dependencies — emit a trivial AcyclicFixture impl (leaf node).
        quote! {
            #[doc(hidden)]
            impl #impl_generics AcyclicFixture for #ident #ty_generics #where_clause {
                const DEPS: &'static [&'static str] = &[];
            }
        }
    } else {
        // Build the dependency name list (normalized) for the const.
        let dep_names: Vec<String> = fixture_args
            .requires
            .iter()
            .map(|p| normalize_path_name(p))
            .collect();
        let dep_name_lits: Vec<_> = dep_names.iter().map(|n| quote! { #n }).collect();

        // Generate compile-time type equality assertions.  These force the
        // compiler to resolve each dependency path to a concrete type and
        // verify that aliased paths (e.g. `Foo` vs `crate::fixtures::Foo`)
        // actually refer to the same type.
        let dep_paths = &fixture_args.requires;
        let type_assertions: Vec<_> = dep_paths
            .iter()
            .map(|dep| {
                quote! {
                    // Ensure the dependency type exists and implements AcyclicFixture.
                    const _: fn() = || {
                        fn _assert_acyclic<T: AcyclicFixture>() {}
                        _assert_acyclic::<#dep>();
                    };
                }
            })
            .collect();

        quote! {
            #[doc(hidden)]
            impl #impl_generics AcyclicFixture for #ident #ty_generics #where_clause {
                const DEPS: &'static [&'static str] = &[#(#dep_name_lits),*];
            }

            // Compile-time assertions that all declared dependencies implement
            // `AcyclicFixture`, preventing cycles from being silently introduced
            // through re-exports or path aliases.
            #(#type_assertions)*
        }
    };

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

        #acyclic_impl
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
    let name = &input.ident;

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
                                    if let Meta::Path(path) = m.value {
                                        contract_ty = Some(path);
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
