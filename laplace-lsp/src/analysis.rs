//! Shallow, per-file analysis shared by semantic tokens, completion, and
//! (partly) diagnostics: a symbol table of declared identifiers -> the block
//! role they were declared in, the set of user-defined function names, and
//! every `pkg::func` call site (reusing `laplace::codegen::rename` directly).
//!
//! This is explicitly NOT a type checker: declared names and their owning
//! block are all that's extracted, never types, dimensions, or constraints.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use laplace::codegen::rename::{find_qualified_calls, QualifiedCall};
use laplace::parser::brace_match::{is_ident_char, CodeMask};
use laplace::parser::library_block::{parse_library_block, ImportStatement};

/// Semantic token legend, in the exact order the client sees it -- indices
/// below must stay in sync with this array.
pub const TOKEN_TYPES: &[&str] = &[
    "laplaceData",
    "laplaceParameter",
    "laplaceTransformedParameter",
    "laplaceGeneratedQuantity",
    "laplaceLocal",
    "laplaceBuiltinFunction",
    "laplaceLibraryFunction",
    "laplaceUserFunction",
];

pub const TOKEN_DATA: u32 = 0;
pub const TOKEN_PARAMETER: u32 = 1;
pub const TOKEN_TRANSFORMED_PARAMETER: u32 = 2;
pub const TOKEN_GENERATED_QUANTITY: u32 = 3;
pub const TOKEN_LOCAL: u32 = 4;
pub const TOKEN_BUILTIN_FUNCTION: u32 = 5;
pub const TOKEN_LIBRARY_FUNCTION: u32 = 6;
pub const TOKEN_USER_FUNCTION: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Data,
    Parameter,
    TransformedParameter,
    GeneratedQuantity,
    Local,
}

impl Role {
    pub fn token_type(self) -> u32 {
        match self {
            Role::Data => TOKEN_DATA,
            Role::Parameter => TOKEN_PARAMETER,
            Role::TransformedParameter => TOKEN_TRANSFORMED_PARAMETER,
            Role::GeneratedQuantity => TOKEN_GENERATED_QUANTITY,
            Role::Local => TOKEN_LOCAL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Functions,
    Data,
    TransformedData,
    Parameters,
    TransformedParameters,
    Model,
    GeneratedQuantities,
    Library,
}

#[derive(Debug, Clone, Copy)]
pub struct Block {
    pub kind: BlockKind,
    pub open_brace: usize,
    pub close_brace: usize,
}

/// `transformed data` must be tried before bare `data`, and `transformed
/// parameters` before bare `parameters` -- see `match_phrase`'s doc comment
/// for why the order among these otherwise doesn't matter.
const BLOCK_KEYWORDS: &[(&[&str], BlockKind)] = &[
    (&["functions"], BlockKind::Functions),
    (&["transformed", "data"], BlockKind::TransformedData),
    (&["data"], BlockKind::Data),
    (&["transformed", "parameters"], BlockKind::TransformedParameters),
    (&["parameters"], BlockKind::Parameters),
    (&["model"], BlockKind::Model),
    (&["generated", "quantities"], BlockKind::GeneratedQuantities),
    (&["library"], BlockKind::Library),
];

fn role_for_block(kind: BlockKind) -> Option<Role> {
    match kind {
        BlockKind::Data => Some(Role::Data),
        BlockKind::Parameters => Some(Role::Parameter),
        // Stan has no separate "role" for transformed data in the spec's
        // enum -- it behaves like precomputed local data, so it maps to
        // `Local` rather than getting its own color.
        BlockKind::TransformedData => Some(Role::Local),
        BlockKind::TransformedParameters => Some(Role::TransformedParameter),
        BlockKind::GeneratedQuantities => Some(Role::GeneratedQuantity),
        BlockKind::Functions | BlockKind::Model | BlockKind::Library => None,
    }
}

/// Find every top-level `<keyword(s)> { ... }` block in `source`. Since Stan
/// blocks never nest inside one another, a single linear scan that jumps
/// past each matched block's closing brace is sufficient and never looks
/// inside a block for further keywords.
pub fn find_top_level_blocks(source: &str, mask: &CodeMask) -> Vec<Block> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;

    'outer: while i < bytes.len() {
        if mask.is_real(i) {
            for (words, kind) in BLOCK_KEYWORDS {
                if let Some(after_kw) = match_phrase(source, mask, i, words) {
                    let brace_pos = skip_real_ws(source, mask, after_kw);
                    if brace_pos < bytes.len() && mask.is_real(brace_pos) && bytes[brace_pos] == b'{' {
                        if let Some(close) = mask.match_closing_brace(source, brace_pos) {
                            out.push(Block {
                                kind: *kind,
                                open_brace: brace_pos,
                                close_brace: close,
                            });
                            i = close + 1;
                            continue 'outer;
                        }
                    }
                }
            }
        }
        i += 1;
    }

    out
}

/// Try to match `words` (each a whole word, separated by required real
/// whitespace) starting exactly at `at`. Returns the byte offset just past
/// the last word on success.
fn match_phrase(source: &str, mask: &CodeMask, at: usize, words: &[&str]) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = at;

    for (idx, word) in words.iter().enumerate() {
        if idx > 0 {
            let ws_start = pos;
            while pos < bytes.len() && mask.is_real(pos) && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos == ws_start {
                return None;
            }
        }

        let wb = word.as_bytes();
        if pos + wb.len() > bytes.len() || &bytes[pos..pos + wb.len()] != wb {
            return None;
        }
        for i in pos..pos + wb.len() {
            if !mask.is_real(i) {
                return None;
            }
        }

        let before_ok = pos == 0 || !is_ident_char(bytes[pos - 1]);
        let end = pos + wb.len();
        let after_ok = end == bytes.len() || !is_ident_char(bytes[end]);
        if !before_ok || !after_ok {
            return None;
        }

        pos = end;
    }

    Some(pos)
}

fn skip_real_ws(source: &str, mask: &CodeMask, mut i: usize) -> usize {
    let bytes = source.as_bytes();
    while i < bytes.len() && mask.is_real(i) && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Stan/laplace type keywords that can start a variable declaration.
/// Compound types (`array[...] real x`) chain more than one of these.
const STAN_TYPE_KEYWORDS: &[&str] = &[
    "int",
    "real",
    "complex",
    "vector",
    "row_vector",
    "matrix",
    "complex_vector",
    "complex_row_vector",
    "complex_matrix",
    "array",
    "ordered",
    "positive_ordered",
    "simplex",
    "unit_vector",
    "cholesky_factor_corr",
    "cholesky_factor_cov",
    "corr_matrix",
    "cov_matrix",
    "void",
    "tuple",
];

fn is_type_keyword(word: &str) -> bool {
    STAN_TYPE_KEYWORDS.contains(&word)
}

/// Split a block's body into top-level statements. A statement ends either
/// at a `;` seen at bracket depth 0, or immediately after a `}` that returns
/// depth to 0 (closing a compound statement like `for (...) { ... }`, which
/// has no trailing `;` of its own). `)`/`]` never end a statement even when
/// they return depth to 0 -- only `}` does.
fn split_top_level_statements(source: &str, mask: &CodeMask, range: Range<usize>) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = range.start;
    let mut i = range.start;

    while i < range.end {
        if mask.is_real(i) {
            match bytes[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' => depth -= 1,
                b'}' => {
                    depth -= 1;
                    if depth <= 0 {
                        out.push(start..i + 1);
                        start = i + 1;
                        depth = 0;
                    }
                }
                b';' if depth == 0 => {
                    out.push(start..i);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if start < range.end {
        out.push(start..range.end);
    }

    out
}

/// Render `range` of `source` with every comment/string byte (per `mask`)
/// replaced by a space, so a shallow textual parse never has to special-case
/// them. Safe because every `is_real` byte in valid Stan/laplace source is
/// ASCII (identifiers, digits, brackets, operators).
fn visible_text(source: &str, mask: &CodeMask, range: Range<usize>) -> String {
    let bytes = source.as_bytes();
    let mut s = String::with_capacity(range.len());
    for i in range {
        if mask.is_real(i) {
            s.push(bytes[i] as char);
        } else {
            s.push(' ');
        }
    }
    s
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Find the byte offset just past the bracket matching `bytes[open_at]`
/// (which must be `open`), scanning for `close` at the same nesting depth.
fn skip_balanced(bytes: &[u8], open_at: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut j = open_at;
    while j < bytes.len() {
        if bytes[j] == open {
            depth += 1;
        } else if bytes[j] == close {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        }
        j += 1;
    }
    None
}

/// If `text` (a single declaration-shaped statement, comments/strings
/// already blanked) starts with a Stan type keyword, walk past any
/// `<constraint>` / `[dims]` groups and chained element-type keywords
/// (`array[N] real x`) to find the declared name. Returns `None` for
/// anything that doesn't look like a declaration (assignments, for/while/if,
/// bare function calls) -- those are left for the generic identifier walk to
/// tag using whatever role their names already have.
fn declared_name(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = skip_ws(bytes, 0);

    let type_start = i;
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    if i == type_start {
        return None;
    }
    if !is_type_keyword(&text[type_start..i]) {
        return None;
    }

    loop {
        i = skip_ws(bytes, i);
        if i >= bytes.len() {
            return None;
        }
        match bytes[i] {
            b'<' => i = skip_balanced(bytes, i, b'<', b'>')?,
            b'[' => i = skip_balanced(bytes, i, b'[', b']')?,
            b if is_ident_char(b) => {
                let word_start = i;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let word = &text[word_start..i];
                if is_type_keyword(word) {
                    continue;
                }
                return Some(word.to_string());
            }
            _ => return None,
        }
    }
}

fn scan_declarations(source: &str, mask: &CodeMask, body: Range<usize>) -> Vec<String> {
    split_top_level_statements(source, mask, body)
        .into_iter()
        .filter_map(|stmt| declared_name(&visible_text(source, mask, stmt)))
        .collect()
}

pub struct Analysis {
    pub symbol_roles: HashMap<String, Role>,
    pub user_functions: HashSet<String>,
    pub library_calls: Vec<QualifiedCall>,
    pub imports: Vec<ImportStatement>,
}

/// `source` with every top-level block (keyword, braces and body alike)
/// replaced by spaces, leaving only the text between the blocks.
///
/// Byte offsets and therefore UTF-8 boundaries are preserved -- each block's
/// range is replaced by exactly as many ASCII spaces -- so the result can be
/// scanned with anything that would run on the original.
fn blank_out_blocks(source: &str, blocks: &[Block]) -> String {
    let mut out = source.to_string();
    for block in blocks {
        // The block's own keyword sits before its opening brace; back up to
        // the start of the line so it can't be read as a function header.
        let start = source[..block.open_brace].rfind('\n').map_or(0, |nl| nl + 1);
        out.replace_range(start..=block.close_brace, &" ".repeat(block.close_brace + 1 - start));
    }
    out
}

pub fn analyze(source: &str) -> Analysis {
    let mask = CodeMask::new(source);
    let blocks = find_top_level_blocks(source, &mask);

    let mut symbol_roles = HashMap::new();
    for block in &blocks {
        if let Some(role) = role_for_block(block.kind) {
            for name in scan_declarations(source, &mask, block.open_brace + 1..block.close_brace) {
                symbol_roles.entry(name).or_insert(role);
            }
        }
    }

    // `extract_signatures` expects a flat sequence of top-level function
    // definitions (the shape of a package's own `.stan` file) -- run it on
    // the `functions { }` block's *body*, not the whole file, since
    // otherwise the block's own opening brace is mistaken for a (headerless,
    // thus skipped) function and its contents are never looked at.
    let mut user_functions: HashSet<String> = blocks
        .iter()
        .find(|b| b.kind == BlockKind::Functions)
        .map(|b| laplace::parser::signatures::extract_signatures(&source[b.open_brace + 1..b.close_brace]))
        .unwrap_or_default()
        .into_iter()
        .map(|sig| sig.name)
        .collect();

    // Plus any function defined bare at top level, outside every block --
    // the shape a `.laplacelib` file is allowed to have (and the only shape
    // it has when it skips the optional `functions { }` wrapper). Running
    // the same scan over the text *between* the blocks costs nothing on a
    // `.laplace` file, where that text is only whitespace and comments.
    user_functions.extend(
        laplace::parser::signatures::extract_signatures(&blank_out_blocks(source, &blocks))
            .into_iter()
            .map(|sig| sig.name),
    );

    let library_calls = find_qualified_calls(source);

    let imports = match parse_library_block(source) {
        Ok(Some(block)) => block.imports,
        _ => Vec::new(),
    };

    Analysis {
        symbol_roles,
        user_functions,
        library_calls,
        imports,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RawToken {
    pub start: usize,
    pub end: usize,
    pub token_type: u32,
}

/// Walk the whole token stream and tag every identifier occurrence (not
/// just declaration sites) with its role or function-origin token type.
pub fn collect_tokens(source: &str, analysis: &Analysis) -> Vec<RawToken> {
    let mask = CodeMask::new(source);
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();

    let mut consumed: Vec<Range<usize>> = Vec::new();
    for call in &analysis.library_calls {
        tokens.push(RawToken {
            start: call.range.start,
            end: call.range.end,
            token_type: TOKEN_LIBRARY_FUNCTION,
        });
        consumed.push(call.range.clone());
    }

    let mut i = 0usize;
    while i < bytes.len() {
        if mask.is_real(i) && is_ident_char(bytes[i]) && (i == 0 || !is_ident_char(bytes[i - 1])) {
            let start = i;
            let mut end = i;
            while end < bytes.len() && is_ident_char(bytes[end]) {
                end += 1;
            }
            i = end;

            if consumed.iter().any(|r| r.start <= start && start < r.end) {
                continue;
            }

            let name = &source[start..end];

            let mut after = end;
            while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                after += 1;
            }
            let is_call = after < bytes.len() && bytes[after] == b'(';

            if is_call {
                if analysis.user_functions.contains(name) {
                    tokens.push(RawToken {
                        start,
                        end,
                        token_type: TOKEN_USER_FUNCTION,
                    });
                } else if crate::builtins::is_builtin(name) {
                    tokens.push(RawToken {
                        start,
                        end,
                        token_type: TOKEN_BUILTIN_FUNCTION,
                    });
                }
            } else if let Some(role) = analysis.symbol_roles.get(name) {
                tokens.push(RawToken {
                    start,
                    end,
                    token_type: role.token_type(),
                });
            }
        } else {
            i += 1;
        }
    }

    tokens.sort_by_key(|t| (t.start, t.end));
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPS_MODEL: &str = r#"library {
  import gps
}

data {
  int<lower=1> N;
  vector[N] x;
  vector[N] y;
}

transformed data {
  real x_mean = mean(x);
}

parameters {
  real<lower=0> alpha;
  real<lower=0> rho;
  real<lower=0> sigma;
}

transformed parameters {
  matrix[N, N] K = gps::rbf_cov(x, alpha, rho);
}

model {
  alpha ~ normal(0, 1);
  rho ~ normal(0, 1);
  sigma ~ normal(0, 1);
  y ~ multi_normal(rep_vector(0, N), K);
}

generated quantities {
  array[N] real y_rep;
  for (n in 1:N) {
    y_rep[n] = normal_rng(alpha, sigma);
  }
}
"#;

    #[test]
    fn declares_names_with_the_right_role_per_block() {
        let analysis = analyze(GPS_MODEL);
        assert_eq!(analysis.symbol_roles.get("N"), Some(&Role::Data));
        assert_eq!(analysis.symbol_roles.get("x"), Some(&Role::Data));
        assert_eq!(analysis.symbol_roles.get("y"), Some(&Role::Data));
        assert_eq!(analysis.symbol_roles.get("x_mean"), Some(&Role::Local));
        assert_eq!(analysis.symbol_roles.get("alpha"), Some(&Role::Parameter));
        assert_eq!(analysis.symbol_roles.get("rho"), Some(&Role::Parameter));
        assert_eq!(
            analysis.symbol_roles.get("K"),
            Some(&Role::TransformedParameter)
        );
        assert_eq!(
            analysis.symbol_roles.get("y_rep"),
            Some(&Role::GeneratedQuantity)
        );
    }

    #[test]
    fn model_block_contributes_no_declarations() {
        let analysis = analyze(GPS_MODEL);
        // `model {}` isn't scanned for declarations -- only usages of names
        // already declared elsewhere.
        assert!(!analysis.symbol_roles.contains_key("n"));
    }

    #[test]
    fn library_call_is_recorded_and_classified_separately_from_roles() {
        let analysis = analyze(GPS_MODEL);
        assert_eq!(analysis.library_calls.len(), 1);
        assert_eq!(analysis.library_calls[0].package, "gps");
        assert_eq!(analysis.library_calls[0].func, "rbf_cov");
    }

    #[test]
    fn imports_are_extracted() {
        let analysis = analyze(GPS_MODEL);
        assert_eq!(analysis.imports.len(), 1);
        assert_eq!(analysis.imports[0].name, "gps");
    }

    #[test]
    fn tokens_tag_usages_inside_model_block_not_just_declarations() {
        let analysis = analyze(GPS_MODEL);
        let tokens = collect_tokens(GPS_MODEL, &analysis);

        // Every occurrence of `alpha` (declared as a parameter) should be
        // tagged, including its two usages inside `model {}`.
        let alpha_occurrences = GPS_MODEL.matches("alpha").count();
        let alpha_tokens = tokens
            .iter()
            .filter(|t| &GPS_MODEL[t.start..t.end] == "alpha" && t.token_type == TOKEN_PARAMETER)
            .count();
        assert_eq!(alpha_tokens, alpha_occurrences);
    }

    #[test]
    fn builtin_and_user_and_library_functions_are_classified_distinctly() {
        let source = r#"library {
  import gps
}
functions {
  real helper(real x) {
    return x * 2;
  }
}
model {
  real y = gps::rbf_cov(1.0, 2.0, 3.0)[1, 1];
  real z = helper(1.0);
  real w = normal_rng(0, 1);
}
"#;
        let analysis = analyze(source);
        let tokens = collect_tokens(source, &analysis);

        assert!(tokens
            .iter()
            .any(|t| &source[t.start..t.end] == "gps::rbf_cov" && t.token_type == TOKEN_LIBRARY_FUNCTION));
        assert!(tokens
            .iter()
            .any(|t| &source[t.start..t.end] == "helper" && t.token_type == TOKEN_USER_FUNCTION));
        assert!(tokens
            .iter()
            .any(|t| &source[t.start..t.end] == "normal_rng" && t.token_type == TOKEN_BUILTIN_FUNCTION));
    }

    #[test]
    fn declared_name_handles_constraints_dims_and_old_style_array_syntax() {
        assert_eq!(declared_name("int<lower=1> N"), Some("N".to_string()));
        assert_eq!(declared_name("vector[N] x"), Some("x".to_string()));
        assert_eq!(declared_name("array[N] real x"), Some("x".to_string()));
        assert_eq!(declared_name("matrix[N, N] K"), Some("K".to_string()));
        assert_eq!(declared_name("real x[3]"), Some("x".to_string()));
        assert_eq!(
            declared_name("real<lower=0, upper=1> theta = 0.5"),
            Some("theta".to_string())
        );
        assert_eq!(declared_name("mu = alpha + beta"), None);
        assert_eq!(declared_name("for (n in 1:N)"), None);
    }

    #[test]
    fn statements_after_a_for_loop_are_still_split_correctly() {
        let body = "vector[N] y_rep;\nfor (n in 1:N) {\n  y_rep[n] = 1;\n}\nreal z;\n";
        let mask = CodeMask::new(body);
        let names = scan_declarations(body, &mask, 0..body.len());
        assert_eq!(names, vec!["y_rep".to_string(), "z".to_string()]);
    }

    #[test]
    fn bare_top_level_function_definitions_are_user_functions() {
        // The `.laplacelib` shape: no `functions { }` wrapper at all.
        let source = "library {\n  import gps\n}\n\nreal f(real x) {\n  return x;\n}\n\nvector g(vector x) {\n  return x;\n}\n";
        let analysis = analyze(source);
        assert!(analysis.user_functions.contains("f"));
        assert!(analysis.user_functions.contains("g"));
        // The import block is not a function definition.
        assert!(!analysis.user_functions.contains("library"));
    }

    #[test]
    fn bare_and_wrapped_definitions_are_both_found_in_one_file() {
        let source = "functions {\n  real inner(real x) {\n    return x;\n  }\n}\n\nreal outer(real x) {\n  return inner(x);\n}\n";
        let analysis = analyze(source);
        assert!(analysis.user_functions.contains("inner"));
        assert!(analysis.user_functions.contains("outer"));
    }

    #[test]
    fn a_laplace_file_gains_no_spurious_user_functions_from_its_blocks() {
        // Nothing outside the blocks of a project file is a definition --
        // scanning that region must stay silent rather than mistaking a
        // block keyword or a statement inside one for a function header.
        let source = "data {\n  int N;\n}\nparameters {\n  real mu;\n}\nmodel {\n  for (n in 1:N) {\n    mu ~ normal(0, 1);\n  }\n}\n";
        let analysis = analyze(source);
        assert!(
            analysis.user_functions.is_empty(),
            "expected no user functions, got {:?}",
            analysis.user_functions
        );
    }
}
