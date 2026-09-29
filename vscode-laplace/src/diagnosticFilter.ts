//! Hiding the diagnostics that patch-1 syntax provokes -- and only those.
//!
//! The Laplace compiler does not accept patch-1 syntax yet, so a library being
//! rewritten in it draws diagnostics that are about the compiler's age rather
//! than about the code. The difficulty is that the tools reporting them also
//! report real problems, and a filter that throws out a whole source loses
//! those too: dropping every `stanc` diagnostic means a missing `;` stops
//! being flagged, which is worse than the noise it removes.
//!
//! So nothing is filtered by source alone. Each rule is anchored to the
//! syntax that actually causes the false positive:
//!
//! - `forbidden-block` is suppressed only on a line that is a `@macro`
//!   header, where `in model { ... }` puts a block keyword before a brace at
//!   depth 0. A real `model` block in a `.laplacelib` is still reported.
//! - `unknown-export` is suppressed only on a line carrying `@use` or
//!   `@expand`, whose name is a template or macro rather than a function
//!   export. A mistyped `pkg::func` anywhere else is still reported.
//! - `stanc` diagnostics are suppressed only from the first line containing
//!   patch-1 syntax onward. `stanc` stops at its first error, so anything it
//!   reports *before* that line was found in Stan it could still parse and is
//!   a genuine error -- a missing `;` above a template keeps its squiggle.
//!
//! The one thing no client-side filter can recover: when patch-1 syntax comes
//! first in a file, `stanc` stops there and never parses what follows, so
//! ordinary Stan mistakes further down go unreported. That is `stanc` bailing
//! out, not this filter hiding anything, and it ends when the compiler learns
//! the syntax.
//!
//! Deliberately free of any `vscode` import, so it can be unit-tested in
//! plain Node.

/// `laplace-lsp` diagnostic codes (see its `diagnostics::code`) that patch-1
/// syntax can provoke. Everything else it reports -- unresolved imports,
/// version conflicts, uninstalled packages -- stays visible always, being
/// true whatever the file's syntax looks like.
const FORBIDDEN_BLOCK = "forbidden-block";
const UNKNOWN_EXPORT = "unknown-export";
const STANC = "stanc";

/// Patch-1 constructs, as they appear on one line of source. A line matching
/// any of these is one the current compiler cannot get past.
const UNSUPPORTED_SYNTAX: readonly RegExp[] = [
  /@(?:template|macro|use|expand|wait)\b/,
  /^\s*pub\b/,
  /\bfunc\s*\(/,
  // `$name` and `${name}` placeholders.
  /\$\{?[A-Za-z_]/,
  // A sized return type: `vector[2] to_pair(real x) {`. `array` is left out
  // on purpose -- `array[] real f(...)` is ordinary Stan. A declaration like
  // `vector[N] mu = ...;` does not match, since an identifier followed by `(`
  // is required.
  new RegExp(
    "^\\s*(?:pub\\s+)?(?:int|real|complex|vector|row_vector|matrix|complex_vector" +
      "|complex_row_vector|complex_matrix|ordered|positive_ordered|simplex|unit_vector" +
      "|cholesky_factor_corr|cholesky_factor_cov|corr_matrix|cov_matrix|tuple)" +
      "\\s*\\[[^\\]]*\\]\\s+[A-Za-z_]\\w*\\s*\\(",
  ),
];

/// Strip a line comment, so a doc comment mentioning `@template` or holding
/// LaTeX with a `$` in it is not mistaken for patch-1 code.
function withoutComment(line: string): string {
  const at = line.indexOf("//");
  return at === -1 ? line : line.slice(0, at);
}

/// Whether one line of source carries patch-1 syntax.
export function lineHasUnsupportedSyntax(line: string): boolean {
  const code = withoutComment(line);
  return UNSUPPORTED_SYNTAX.some((re) => re.test(code));
}

/// The 0-based index of the first line carrying patch-1 syntax, or
/// `undefined` if the document has none -- in which case nothing at all is
/// suppressed and the file behaves exactly as it did before this feature
/// existed.
export function firstUnsupportedLine(text: string): number | undefined {
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    if (lineHasUnsupportedSyntax(lines[i])) {
      return i;
    }
  }
  return undefined;
}

/// The part of `vscode.Diagnostic` this needs, so the module stays testable
/// without the editor API.
export interface CodedDiagnostic {
  readonly source?: string;
  readonly code?: string | number | { readonly value: string | number };
  readonly range: { readonly start: { readonly line: number } };
}

/// What the filter needs to know about the document a diagnostic came from.
export interface DocumentContext {
  /// Result of [`firstUnsupportedLine`] for this document.
  readonly firstUnsupportedLine?: number;
  /// The text of one 0-based line, or `undefined` if out of range.
  lineAt(line: number): string | undefined;
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

/// Whether this diagnostic is one patch-1 syntax provokes, given where it
/// landed in the document.
export function isUnsupportedSyntaxNoise(
  diagnostic: CodedDiagnostic,
  context: DocumentContext,
): boolean {
  const line = diagnostic.range.start.line;

  if (diagnostic.source === STANC) {
    const first = context.firstUnsupportedLine;
    return first !== undefined && line >= first;
  }

  const code = codeOf(diagnostic);
  const text = context.lineAt(line) ?? "";

  if (code === FORBIDDEN_BLOCK) {
    // Only the `in <blocks> {` of a macro header is a false positive.
    return /@macro\b/.test(withoutComment(text));
  }
  if (code === UNKNOWN_EXPORT) {
    return /@(?:use|expand)\b/.test(withoutComment(text));
  }
  return false;
}

/// `diagnostics` with the patch-1 noise removed, or the same array back when
/// `show` is true so the normal path allocates nothing.
export function filterDiagnostics<T extends CodedDiagnostic>(
  diagnostics: T[],
  show: boolean,
  context: DocumentContext,
): T[] {
  if (show || diagnostics.length === 0) {
    return diagnostics;
  }
  return diagnostics.filter((d) => !isUnsupportedSyntaxNoise(d, context));
}
