//! Structured diagnostics for `#[bffi]` rejections.
//!
//! Every diagnostic is rendered by the shared
//! [`crate::support::diagnostics::MacroDiagnostic`]; this module
//! keeps this crate's constructors with their stable codes and exact
//! message texts. Stable codes (do not renumber; UI goldens in
//! `tests/ui` lock them):
//!
//! | Code   | Meaning                                                 |
//! |--------|---------------------------------------------------------|
//! | `E001` | unsupported function shape (async/generic/unsafe/...)   |
//! | `E002` | unsupported parameter type                              |
//! | `E003` | unsupported return type                                 |
//! | `E004` | unsupported attribute options (only `crate = "..."`)    |

use crate::support::diagnostics::{DESIGN_NOTE, MacroDiagnostic};
use proc_macro2::Span;
use quote::ToTokens;

/// The accepted parameter set, listed in every type-rejection help
/// line.
const PARAM_TYPES: &str = "supported: i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|&str|&[u8]|()";
/// The accepted return set: parameters plus the P2 buffer payloads and
/// the `Result` error channel.
const RETURN_TYPES: &str = "supported returns: parameters|String|Vec<u8>|CopiedBuf|Option<buffer>|Result<T, E: Error + Send + Sync>";

/// Adds the help/note lines shared by all shape-style diagnostics.
fn shape_guidance(diag: MacroDiagnostic) -> MacroDiagnostic {
    diag.with_help("plain `fn`s only in P1")
        .with_note(DESIGN_NOTE)
}

/// `E001` - the function shape is outside the P1 rules (`async`,
/// generic, `unsafe`, method receiver, variadic, `extern`, `const`).
/// `what` names the shape; anchored at the offending tokens' span.
pub(crate) fn fn_shape(span: Span, what: &str) -> syn::Error {
    shape_guidance(MacroDiagnostic::new(
        "E001",
        format!("unsupported function shape: {what}"),
    ))
    .to_compile_error(span)
}

/// `E001` - the parameter pattern is outside the P1 rules: only
/// identifiers and `_` name a boundary parameter. Anchored at the
/// offending pattern.
pub(crate) fn param_pattern(span: Span) -> syn::Error {
    MacroDiagnostic::new(
        "E001",
        "unsupported function shape: non-identifier parameter pattern",
    )
    .with_help("use `name: Type` or `_`: Type")
    .with_note(DESIGN_NOTE)
    .to_compile_error(span)
}

/// `E002` - a parameter type outside the accepted boundary set,
/// anchored at the offending type.
pub(crate) fn param_type<T: ToTokens>(span: Span, ty_tokens: &T, name: &str) -> syn::Error {
    MacroDiagnostic::new(
        "E002",
        format!(
            "unsupported type `{}` for parameter `{}`",
            ty_tokens.to_token_stream(),
            name,
        ),
    )
    .with_help(PARAM_TYPES)
    .with_note(
        "borrowed `&[u8]` is the only buffer parameter; owned buffers are return-only (CALLING-CONVENTION.md)",
    )
    .with_note(DESIGN_NOTE)
    .to_compile_error(span)
}

/// `E003` - a return type outside the accepted boundary set, anchored
/// at the offending type.
pub(crate) fn return_type<T: ToTokens>(span: Span, ty_tokens: &T) -> syn::Error {
    MacroDiagnostic::new(
        "E003",
        format!(
            "unsupported type `{}` for the return type",
            ty_tokens.to_token_stream(),
        ),
    )
    .with_help(RETURN_TYPES)
    .with_note("Option covers buffer payloads only; `E` in `Result` must impl `std::error::Error + Send + Sync`")
    .with_note(DESIGN_NOTE)
    .to_compile_error(span)
}

/// `E004` - the attribute carries unsupported options. The only
/// option is `crate = "<name>"` (facade-only mode); anything else -
/// an unknown key, a non-literal or invalid `crate` value - lands
/// here. Anchored at the attribute tokens' span.
pub(crate) fn attr_options(span: Span) -> syn::Error {
    shape_guidance(MacroDiagnostic::new(
        "E004",
        "unknown option; only `crate = \"...\"` is supported",
    ))
    .to_compile_error(span)
}

/// `E002` - a parameter type an ASYNC function cannot accept:
/// borrowed parameters cannot cross the spawn boundary. Anchored at
/// the offending type.
pub(crate) fn async_param_type<T: ToTokens>(span: Span, ty_tokens: &T, name: &str) -> syn::Error {
    MacroDiagnostic::new(
        "E002",
        format!(
            "unsupported type `{}` for async parameter `{}`",
            ty_tokens.to_token_stream(),
            name,
        ),
    )
    .with_help("supported async parameters: i8|i16|i32|u8|u16|u32|f32|f64|bool|String|Vec<u8> (owned - `&str`/`&[u8]` cannot cross spawn)")
    .with_note(DESIGN_NOTE)
    .to_compile_error(span)
}

#[cfg(test)]
mod tests {
    use super::{MacroDiagnostic, param_type, return_type};
    use proc_macro2::Span;
    use quote::quote;

    #[test]
    fn param_type_diagnostic_renders_code_help_and_notes() {
        let err = param_type(Span::call_site(), &quote! { Vec<u8> }, "data");
        let text = err.to_string();
        assert!(
            text.contains("bffi[E002]: unsupported type `Vec < u8 >` for parameter `data`")
                && text.contains(
                    "  = help: supported: i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|&str|&[u8]|()"
                )
                && text.contains(
                    "  = note: borrowed `&[u8]` is the only buffer parameter; owned buffers are return-only (CALLING-CONVENTION.md)"
                )
                && text.contains(
                    "  = note: boundary rules: DESIGN.md (https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)"
                )
        );
    }

    #[test]
    fn return_type_diagnostic_lists_the_p2_return_set() {
        let err = return_type(Span::call_site(), &quote! { Option<i32> });
        let text = err.to_string();
        assert!(text.contains("bffi[E003]: unsupported type `Option < i32 >` for the return type"));
        assert!(text.contains(
            "  = help: supported returns: parameters|String|Vec<u8>|CopiedBuf|Option<buffer>|Result<T, E: Error + Send + Sync>"
        ));
        assert!(text.contains(
            "  = note: Option covers buffer payloads only; `E` in `Result` must impl `std::error::Error + Send + Sync`"
        ));
        // The shared renderer stays usable directly (documented
        // contract: code + message + help/notes).
        let rendered = MacroDiagnostic::new("E001", "probe")
            .to_compile_error(Span::call_site())
            .to_string();
        assert!(rendered.starts_with("bffi[E001]: probe"));
    }
}
