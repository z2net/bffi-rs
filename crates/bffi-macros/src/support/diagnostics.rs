//! The shared diagnostic renderer for the proc-macro crates.
//!
//! Every rejection is a [`MacroDiagnostic`]: a stable code plus a
//! message, actionable help lines and context notes, rendered into a
//! `compile_error!`. This is the compile-time counterpart of the
//! runtime `BffiError` scheme (code + message + source); the runtime
//! mapping of those errors to JS errors is `bffi-error`'s job.
//!
//! The stable codes and the exact texts stay with the consumers: the
//! constructor functions with their E-codes (`E001`-`E004` in
//! `bffi-macros`, `E005`-`E008` in `bffi-class`) live in the
//! proc-macro crates and are locked by the trybuild goldens there.

use proc_macro2::Span;

/// Note pointing at the boundary rules in DESIGN.md (shared by all
/// diagnostics of both proc-macro crates).
pub const DESIGN_NOTE: &str =
    "boundary rules: DESIGN.md (https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)";

/// One structured macro diagnostic: a stable code, a message,
/// actionable help lines and context notes. Rendered to a
/// `compile_error!` - the compile-time counterpart of the runtime
/// `BffiError` (code + message + source) scheme.
pub struct MacroDiagnostic {
    /// Stable code (`E001` .. `E015` so far; each proc-macro crate
    /// owns its series).
    code: &'static str,
    /// First-line message (without the `bffi[code]: ` prefix).
    message: String,
    /// Actionable `  = help: ` lines.
    help: Vec<String>,
    /// Context `  = note: ` lines.
    notes: Vec<String>,
}

impl MacroDiagnostic {
    /// Starts a diagnostic with the given stable code and first-line
    /// message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            help: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Appends an actionable `  = help: ` line.
    pub fn with_help(mut self, line: impl Into<String>) -> Self {
        self.help.push(line.into());
        self
    }

    /// Appends a context `  = note: ` line.
    pub fn with_note(mut self, line: impl Into<String>) -> Self {
        self.notes.push(line.into());
        self
    }

    /// Renders into a `syn::Error` anchored at `span`: a
    /// `bffi[<code>]: <message>` first line, then `  = help: ` per
    /// help line, then `  = note: ` per note.
    // Consuming `self` (and the `to_*` name) is the documented contract
    // for this renderer; the diagnostic is single-use.
    #[allow(clippy::wrong_self_convention)]
    pub fn to_compile_error(self, span: Span) -> syn::Error {
        let mut lines = vec![format!("bffi[{}]: {}", self.code, self.message)];
        lines.extend(self.help.iter().map(|h| format!("  = help: {h}")));
        lines.extend(self.notes.iter().map(|n| format!("  = note: {n}")));
        syn::Error::new(span, lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::{DESIGN_NOTE, MacroDiagnostic};
    use proc_macro2::Span;

    #[test]
    fn renders_code_help_and_notes_in_order() {
        let err = MacroDiagnostic::new("E001", "unsupported function shape: async function")
            .with_help("plain `fn`s only")
            .with_note(DESIGN_NOTE)
            .to_compile_error(Span::call_site());
        let text = err.to_string();
        assert!(text.starts_with("bffi[E001]: unsupported function shape: async function"));
        assert!(text.contains("  = help: plain `fn`s only"));
        assert!(text.ends_with(&format!("  = note: {DESIGN_NOTE}")));
    }

    #[test]
    fn design_note_carries_the_absolute_docs_url() {
        assert_eq!(
            DESIGN_NOTE,
            "boundary rules: DESIGN.md (https://github.com/z2net/bffi-rs/blob/main/docs/DESIGN.md)"
        );
    }
}
