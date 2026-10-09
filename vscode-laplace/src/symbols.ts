//! Math-symbol display: showing `sigma_obs` as σ with a subscript, `sum(x)`
//! as ∑(x) and `<=` as ≤, and the `\sigma`-style input that types them.
//!
//! Display only. The file keeps its plain ASCII names -- neither laplace nor
//! `stanc` accepts `σ` in an identifier -- and the editor draws the symbol over
//! them. Input works the same way round: `\sigma_{obs}` completes to the text
//! `sigma_obs`, which is then drawn as σ_obs.
//!
//! Deliberately free of any `vscode` import, so it can be unit-tested in
//! plain Node.

/// LaTeX's Greek letter commands, by the name a variable carries in the file.
/// `\epsilon` / `\varepsilon` and `\phi` / `\varphi` follow LaTeX (and Julia):
/// the plain command is the less common glyph.
export const GREEK: Readonly<Record<string, string>> = {
  alpha: "α",
  beta: "β",
  gamma: "γ",
  delta: "δ",
  epsilon: "ϵ",
  varepsilon: "ε",
  zeta: "ζ",
  eta: "η",
  theta: "θ",
  vartheta: "ϑ",
  iota: "ι",
  kappa: "κ",
  lambda: "λ",
  mu: "μ",
  nu: "ν",
  xi: "ξ",
  pi: "π",
  varpi: "ϖ",
  rho: "ρ",
  varrho: "ϱ",
  sigma: "σ",
  varsigma: "ς",
  tau: "τ",
  upsilon: "υ",
  phi: "ϕ",
  varphi: "φ",
  chi: "χ",
  psi: "ψ",
  omega: "ω",
  Gamma: "Γ",
  Delta: "Δ",
  Theta: "Θ",
  Lambda: "Λ",
  Xi: "Ξ",
  Pi: "Π",
  Sigma: "Σ",
  Upsilon: "Υ",
  Phi: "Φ",
  Psi: "Ψ",
  Omega: "Ω",
};

/// Stan functions drawn as an operator symbol. Only at a call -- the name
/// followed by `(` -- and the parentheses stay: `sum(x)` reads `∑(x)`.
/// `∑` and `∏` are the n-ary operators, not the letters `Σ` and `Π`, so a
/// covariance matrix called `Sigma` still looks different from a sum.
export const CALLS: Readonly<Record<string, string>> = {
  sum: "∑",
  prod: "∏",
  sqrt: "√",
  pi: "π",
};

/// Comparison operators and their symbols.
export const OPERATORS: Readonly<Record<string, string>> = {
  "<=": "≤",
  ">=": "≥",
  "!=": "≠",
};

/// One stretch of the file to draw differently.
export interface SymbolSpan {
  /// Byte range in the text, `end` exclusive -- hidden and drawn over.
  start: number;
  end: number;
  /// What is drawn in its place.
  symbol: string;
  /// Drawn after `symbol`, lowered: the `obs` of `sigma_obs`.
  subscript?: string;
}

/// Which characters of `text` are code, as opposed to a comment or a string
/// literal. Symbols are never drawn in either: a `//@math` comment is LaTeX
/// in its own right, and a string is printed exactly as written.
export function codeMask(text: string): Uint8Array {
  const mask = new Uint8Array(text.length);
  let i = 0;
  while (i < text.length) {
    if (text.startsWith("//", i)) {
      const end = text.indexOf("\n", i);
      i = end === -1 ? text.length : end;
      continue;
    }
    if (text.startsWith("/*", i)) {
      const end = text.indexOf("*/", i + 2);
      i = end === -1 ? text.length : end + 2;
      continue;
    }
    if (text[i] === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"' && text[j] !== "\n") {
        j += text[j] === "\\" ? 2 : 1;
      }
      i = j + 1;
      continue;
    }
    mask[i] = 1;
    i += 1;
  }
  return mask;
}

const IDENTIFIER = /[A-Za-z_][A-Za-z0-9_]*/g;
const OPERATOR = /<=|>=|!=/g;

/// Every span to draw as a symbol, in order.
///
/// A name is drawn only where it is a plain identifier in code. Never after
/// `pkg::` or `.`, never as a `$placeholder` or inside `${...}`, never as a
/// package name before `::`. And a Greek name is never drawn at a call, so
/// Stan's `beta(...)` and `gamma(...)` keep their names: only the functions in
/// [`CALLS`] become symbols there.
export function findSymbols(text: string): SymbolSpan[] {
  const mask = codeMask(text);
  const spans: SymbolSpan[] = [];

  IDENTIFIER.lastIndex = 0;
  for (let m = IDENTIFIER.exec(text); m; m = IDENTIFIER.exec(text)) {
    const start = m.index;
    const end = start + m[0].length;
    if (!mask[start] || !isPlainIdentifier(text, start, end)) {
      continue;
    }
    if (isCall(text, end)) {
      const symbol = CALLS[m[0]];
      if (symbol) {
        spans.push({ start, end, symbol });
      }
      continue;
    }
    const greek = greekName(m[0]);
    if (greek) {
      spans.push({ start, end, ...greek });
    }
  }

  OPERATOR.lastIndex = 0;
  for (let m = OPERATOR.exec(text); m; m = OPERATOR.exec(text)) {
    if (mask[m.index] && mask[m.index + 1]) {
      spans.push({ start: m.index, end: m.index + 2, symbol: OPERATORS[m[0]] });
    }
  }

  return spans.sort((a, b) => a.start - b.start);
}

/// `sigma` -> σ; `sigma_obs` -> σ with subscript `obs`. The LaTeX reading of
/// a name: the Greek letter, then everything after the first `_` as its
/// subscript, as `\sigma_{obs}` would set it.
export function greekName(name: string): { symbol: string; subscript?: string } | undefined {
  const underscore = name.indexOf("_");
  const base = underscore === -1 ? name : name.slice(0, underscore);
  const symbol = Object.prototype.hasOwnProperty.call(GREEK, base) ? GREEK[base] : undefined;
  if (!symbol) {
    return undefined;
  }
  if (underscore === -1) {
    return { symbol };
  }
  const subscript = name.slice(underscore + 1);
  return subscript ? { symbol, subscript } : undefined;
}

function isPlainIdentifier(text: string, start: number, end: number): boolean {
  const before = text[start - 1];
  if (before === "$" || before === "." || before === "@") {
    return false;
  }
  if (before === "{" && text[start - 2] === "$") {
    return false;
  }
  if (text.startsWith("::", start - 2) || text.startsWith("::", end)) {
    return false;
  }
  return true;
}

function isCall(text: string, end: number): boolean {
  let i = end;
  while (text[i] === " " || text[i] === "\t") {
    i += 1;
  }
  return text[i] === "(";
}

// --- input -----------------------------------------------------------------

/// One `\command` the input offers.
export interface SymbolCompletion {
  /// What the user types, and what the suggestion list shows: `\sigma`.
  label: string;
  /// The symbol, shown beside the label.
  symbol: string;
  /// What goes into the file: `sigma`.
  insert: string;
}

/// Every `\command`, in a stable order: Greek letters, then the operators.
export const COMMANDS: readonly SymbolCompletion[] = [
  ...Object.entries(GREEK)
    // `\pi` is the constant: Stan spells it `pi()`. Listed below.
    .filter(([name]) => name !== "pi")
    .map(([name, symbol]) => ({ label: `\\${name}`, symbol, insert: name })),
  { label: "\\pi", symbol: "π", insert: "pi()" },
  { label: "\\sum", symbol: CALLS.sum, insert: "sum" },
  { label: "\\prod", symbol: CALLS.prod, insert: "prod" },
  { label: "\\sqrt", symbol: CALLS.sqrt, insert: "sqrt" },
  { label: "\\leq", symbol: "≤", insert: "<=" },
  { label: "\\le", symbol: "≤", insert: "<=" },
  { label: "\\geq", symbol: "≥", insert: ">=" },
  { label: "\\ge", symbol: "≥", insert: ">=" },
  { label: "\\neq", symbol: "≠", insert: "!=" },
  { label: "\\ne", symbol: "≠", insert: "!=" },
];

/// A `\command` being typed, LaTeX-style: `\sig`, `\sigma`, `\sigma_y`,
/// `\sigma_{obs`, `\sigma_{obs}`.
const TYPING = /\\([A-Za-z]*)(?:_(?:\{([A-Za-z0-9_]*)(\}?)|([A-Za-z0-9]*)))?$/;

/// What to offer while the user types a `\command`, given the line's text
/// before and after the cursor. `start` and `end` are the columns to replace:
/// from the `\` up to the cursor, plus the `}` an editor auto-closed after it.
///
/// Without a subscript, every command is offered and the editor filters them
/// as the user types. With one -- `\sigma_y` or `\sigma_{obs}`, the two ways
/// LaTeX writes a subscript -- there is exactly one answer, `sigma_obs`, which
/// is drawn back as σ_obs.
export function completeAt(
  before: string,
  after: string,
): { start: number; end: number; items: SymbolCompletion[] } | undefined {
  const m = TYPING.exec(before);
  if (!m) {
    return undefined;
  }
  const [typed, name, braced, closed, bare] = m;
  const start = before.length - typed.length;
  const hasSubscript = braced !== undefined || bare !== undefined;

  if (!hasSubscript) {
    return { start, end: before.length, items: [...COMMANDS] };
  }

  const subscript = braced ?? bare ?? "";
  const symbol = Object.prototype.hasOwnProperty.call(GREEK, name) ? GREEK[name] : undefined;
  if (!symbol || !subscript) {
    return undefined;
  }
  // `\sigma_{obs` with the `}` auto-closed after the cursor: replace it too.
  const autoClosed = braced !== undefined && !closed && after.startsWith("}");
  const label = braced !== undefined ? `\\${name}_{${subscript}}` : `\\${name}_${subscript}`;
  return {
    start,
    end: before.length + (autoClosed ? 1 : 0),
    items: [{ label, symbol: `${symbol}_${subscript}`, insert: `${name}_${subscript}` }],
  };
}
