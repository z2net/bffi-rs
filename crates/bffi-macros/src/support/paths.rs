//! Configurable crate-root paths for the generated code.
//!
//! The generated tokens name the runtime crates by absolute path,
//! because the expansion lands in the user crate. Since the stack
//! merged into the published `bffi` facade, the default roots are
//! the facade's namespaced re-exports: `::bffi::core`,
//! `::bffi::types`, `::bffi::dts`, `::bffi::object`, `::bffi::build`
//! and `::bffi::r#async` - a downstream crate whose only dependency
//! is `bffi` compiles with no extra attributes.
//!
//! The `crate = "<name>"` attribute option (parsed by the
//! proc-macro crates, which own the diagnostics) swaps every root
//! for the namespaced re-exports of another facade:
//! `::<name>::{core, types, dts, object, build, r#async}`.
//! `crate = "bffi"` therefore remains accepted and is exactly the
//! default.
//!
//! The special value `crate = "direct"` selects the pre-merge direct
//! dependency roots (`::bffi_core`, ...): an in-workspace escape
//! hatch for the historical crate names. The pre-merge crates are
//! not published, so downstream code has no reason to use it.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// The six crate roots the generated code names, plus the
/// `crate = "..."` mapping onto a facade's re-export namespaces.
///
/// [`PathCtx::default`] yields the published-facade roots
/// (`::bffi::core`, ...). [`PathCtx::direct`] yields the pre-merge
/// direct-dependency roots. [`PathCtx::from_attr`] yields the
/// namespaced roots of one facade crate.
#[derive(Clone, Debug)]
pub struct PathCtx {
    /// Root for `bffi-core` (ErrorCode, BffiError, Handle, TypeTag,
    /// `boundary::run_extern_body`, `set_last_error`).
    pub core: TokenStream,
    /// Root for `bffi-types` (CopiedBuf, `str_view`, `buf_view`,
    /// `unsafe_zero_copy`).
    pub types: TokenStream,
    /// Root for `bffi-dts` (the descriptor IR types).
    pub dts: TokenStream,
    /// Root for `bffi-object` (ObjectWrap, ObjectError).
    pub object: TokenStream,
    /// Root for `bffi-build` (`runtime::store_bytes`).
    pub build: TokenStream,
    /// Root for `bffi-async` (`spawn`, `AsyncValue`); the facade
    /// re-exports it as the raw-identifier module `r#async` because
    /// `async` is a keyword.
    pub r#async: TokenStream,
}

impl Default for PathCtx {
    fn default() -> Self {
        Self::facade("bffi")
    }
}

impl PathCtx {
    /// The pre-merge direct-dependency roots (`::bffi_core`,
    /// `::bffi_types`, ...). Selected by `crate = "direct"`; see the
    /// module docs for who that is for.
    pub fn direct() -> PathCtx {
        Self {
            core: quote! { ::bffi_core },
            types: quote! { ::bffi_types },
            dts: quote! { ::bffi_dts },
            object: quote! { ::bffi_object },
            build: quote! { ::bffi_build },
            r#async: quote! { ::bffi_async },
        }
    }

    /// Builds the context for a validated `crate = "<name>"` value:
    /// every root becomes `::<name>::<namespace>` (crate names with
    /// hyphens are referenced with underscores, as Rust requires);
    /// the async root becomes the raw-identifier module
    /// `::<name>::r#async`.
    ///
    /// Validate the name with [`is_crate_name`] first; this
    /// constructor assumes a valid name.
    pub fn from_attr(name: &str) -> PathCtx {
        Self::facade(&name.replace('-', "_"))
    }

    /// `from_attr` on an already-normalized root identifier.
    fn facade(root: &str) -> PathCtx {
        let core = format_ident!("{}", root);
        Self {
            core: quote! { ::#core::core },
            types: quote! { ::#core::types },
            dts: quote! { ::#core::dts },
            object: quote! { ::#core::object },
            build: quote! { ::#core::build },
            r#async: quote! { ::#core::r#async },
        }
    }
}

/// Whether `name` is a valid `crate = "<name>"` value: a non-empty
/// crate-ish identifier (ASCII letters, digits, `_`, `-`; not
/// starting with a digit or `-`) that does not shadow a language
/// crate (`core`/`std`/`alloc`) or a path keyword.
pub fn is_crate_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return false;
    }
    // Redirecting the generated absolute paths into the language
    // crates or a keyword would compile to nonsense; reject up front.
    !matches!(
        name,
        "core" | "std" | "alloc" | "crate" | "self" | "super" | "Self"
    )
}

/// Maps a validated `crate = "<name>"` value onto its context: the
/// special value `"direct"` selects [`PathCtx::direct`]; any other
/// valid name goes through [`PathCtx::from_attr`].
pub fn from_option(name: &str) -> PathCtx {
    if name == "direct" {
        PathCtx::direct()
    } else {
        PathCtx::from_attr(name)
    }
}

#[cfg(test)]
mod tests {
    use super::{PathCtx, is_crate_name};

    #[test]
    fn default_tokens_are_the_facade_namespace_paths() {
        let ctx = PathCtx::default();
        assert_eq!(ctx.core.to_string(), ":: bffi :: core");
        assert_eq!(ctx.types.to_string(), ":: bffi :: types");
        assert_eq!(ctx.dts.to_string(), ":: bffi :: dts");
        assert_eq!(ctx.object.to_string(), ":: bffi :: object");
        assert_eq!(ctx.build.to_string(), ":: bffi :: build");
        assert_eq!(ctx.r#async.to_string(), ":: bffi :: r#async");
    }

    #[test]
    fn default_matches_from_attr_for_the_published_facade() {
        // Downstream `#[bffi]` without options and `#[bffi(crate = "bffi")]`
        // must emit byte-identical paths: the attribute stays accepted
        // (0.1.x compatibility) but is no longer required.
        assert_eq!(
            PathCtx::default().core.to_string(),
            PathCtx::from_attr("bffi").core.to_string()
        );
        assert_eq!(
            PathCtx::default().r#async.to_string(),
            PathCtx::from_attr("bffi").r#async.to_string()
        );
    }

    #[test]
    fn direct_yields_the_pre_merge_dependency_roots() {
        let ctx = PathCtx::direct();
        assert_eq!(ctx.core.to_string(), ":: bffi_core");
        assert_eq!(ctx.types.to_string(), ":: bffi_types");
        assert_eq!(ctx.dts.to_string(), ":: bffi_dts");
        assert_eq!(ctx.object.to_string(), ":: bffi_object");
        assert_eq!(ctx.build.to_string(), ":: bffi_build");
        assert_eq!(ctx.r#async.to_string(), ":: bffi_async");
    }

    #[test]
    fn from_attr_maps_onto_the_facade_namespaces() {
        let ctx = PathCtx::from_attr("bffi");
        assert_eq!(ctx.core.to_string(), ":: bffi :: core");
        assert_eq!(ctx.types.to_string(), ":: bffi :: types");
        assert_eq!(ctx.dts.to_string(), ":: bffi :: dts");
        assert_eq!(ctx.object.to_string(), ":: bffi :: object");
        assert_eq!(ctx.build.to_string(), ":: bffi :: build");
        assert_eq!(ctx.r#async.to_string(), ":: bffi :: r#async");
    }

    #[test]
    fn from_attr_normalizes_hyphens_to_underscores() {
        let ctx = PathCtx::from_attr("my-facade");
        assert_eq!(ctx.core.to_string(), ":: my_facade :: core");
    }

    #[test]
    fn from_option_dispatches_direct_and_facade_names() {
        let direct = super::from_option("direct");
        assert_eq!(direct.core.to_string(), PathCtx::direct().core.to_string());
        assert_eq!(
            direct.r#async.to_string(),
            PathCtx::direct().r#async.to_string()
        );

        let facade = super::from_option("my-facade");
        assert_eq!(
            facade.core.to_string(),
            PathCtx::from_attr("my-facade").core.to_string()
        );
    }

    #[test]
    fn crate_names_follow_the_crate_ident_rules() {
        assert!(is_crate_name("bffi"));
        assert!(is_crate_name("my-facade"));
        assert!(is_crate_name("my_facade"));
        assert!(is_crate_name("_private"));
        assert!(is_crate_name("a1"));
        // `crate = "direct"` selects the pre-merge roots; it is a valid
        // value of the option.
        assert!(is_crate_name("direct"));
    }

    #[test]
    fn crate_names_reject_the_language_crates_and_junk() {
        for name in [
            "",
            "core",
            "std",
            "alloc",
            "crate",
            "self",
            "super",
            "Self",
            "1bad",
            "-bad",
            "has space",
            "has.dot",
            "has::path",
        ] {
            assert!(!is_crate_name(name), "`{name}` must be rejected");
        }
    }
}
