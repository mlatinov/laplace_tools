//! Hiding the diagnostics that patch-1 syntax provokes.
//!
//! The Laplace compiler does not accept patch-1 syntax yet, so a library
//! being rewritten in it draws diagnostics that are about the compiler's
//! age, not about the code. `laplace-lsp` codes its diagnostics
//! (`laplace-lsp/src/diagnostics.rs`, `pub mod code`), which is what lets
//! this filter be narrow: unresolved imports, version conflicts and
//! uninstalled packages stay visible, because they are still true whatever
//! the file's syntax looks like.
//!
//! Which diagnostics patch-1 syntax actually provokes is pinned down by
//! tests in `laplace-lsp/src/diagnostics.rs`
//! (`a_macro_headers_in_clause_looks_like_a_forbidden_block`,
//! `a_use_of_a_template_looks_like_an_unexported_function`). If a compiler
//! update changes that set, those fail and this list needs revisiting.
//!
//! Deliberately free of any `vscode` import, so it can be unit-tested in
//! plain Node.

/// `laplace-lsp` diagnostic codes that patch-1 syntax triggers spuriously.
///
/// - `forbidden-block`: a `@macro` header's `in model { ... }` puts
///   `model {` at brace depth 0, where the compiler's block scanner cannot
///   tell it from a real `model` block in a `.laplacelib`.
/// - `unknown-export`: `@use pkg::name` / `@expand pkg::name` name a
///   template or a macro, and neither is a function export.
const SUPPRESSED_CODES: ReadonlySet<string> = new Set(["forbidden-block", "unknown-export"]);

/// Everything from the `stanc` pass. Patch-1 syntax reaches `stanc` verbatim
/// -- `codegen` splices source rather than parsing statements -- so `stanc`
/// rejects the generated model wholesale and every one of its diagnostics is
/// noise. It contributes nothing at all unless `stanc` is installed.
const SUPPRESSED_SOURCE = "stanc";

/// The shape of the part of `vscode.Diagnostic` this needs, so the module
/// stays testable without the editor API.
export interface CodedDiagnostic {
  readonly source?: string;
  readonly code?: string | number | { readonly value: string | number };
}

function codeOf(diagnostic: CodedDiagnostic): string | undefined {
  const { code } = diagnostic;
  if (typeof code === "string") {
    return code;
  }
  if (typeof code === "number") {
    return String(code);
  }
  if (code && typeof code === "object" && "value" in code) {
    return String(code.value);
  }
  return undefined;
}

/// Whether this diagnostic is one patch-1 syntax provokes.
export function isUnsupportedSyntaxNoise(diagnostic: CodedDiagnostic): boolean {
  if (diagnostic.source === SUPPRESSED_SOURCE) {
    return true;
  }
  const code = codeOf(diagnostic);
  return code !== undefined && SUPPRESSED_CODES.has(code);
}

/// `diagnostics` with the patch-1 noise removed, or the same array back when
/// `show` is true so the normal path allocates nothing.
export function filterDiagnostics<T extends CodedDiagnostic>(diagnostics: T[], show: boolean): T[] {
  if (show) {
    return diagnostics;
  }
  return diagnostics.filter((d) => !isUnsupportedSyntaxNoise(d));
}
