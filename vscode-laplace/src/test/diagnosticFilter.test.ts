import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import {
  CodedDiagnostic,
  DocumentContext,
  filterDiagnostics,
  firstUnsupportedLine,
  isUnsupportedSyntaxNoise,
  lineHasUnsupportedSyntax,
} from "../diagnosticFilter";

/// Build a context from document text, the way the extension does.
function ctx(text: string): DocumentContext {
  const lines = text.split("\n");
  return {
    firstUnsupportedLine: firstUnsupportedLine(text),
    lineAt: (l) => lines[l],
  };
}

const at = (line: number, source: string, code?: string): CodedDiagnostic => ({
  source,
  code,
  range: { start: { line } },
});

// --- the reported bug ----------------------------------------------------

/// The case that made the old filter unusable: a file with no patch-1 syntax
/// at all, a missing `;`, and the setting turned off. The missing `;` must
/// still be flagged -- turning off patch-1 noise must not turn off Stan.
test("a missing semicolon is still reported in a file with no patch-1 syntax", () => {
  const text = [
    "real churn_frailty_ll(vector log_h, vector H, real theta, data vector d) {",
    "  return 1",
    "}",
  ].join("\n");

  assert.equal(firstUnsupportedLine(text), undefined, "this file has no patch-1 syntax");

  const diags = [at(2, "stanc")];
  assert.deepEqual(filterDiagnostics(diags, false, ctx(text)), diags, "must not be hidden");
});

/// stanc stops at its first error, so a real error *above* a template is the
/// one it reports -- and it has to survive the filter.
test("a real error before the first patch-1 line survives", () => {
  const text = [
    "real broken(real x) {", // 0
    "  return 1", //            1
    "}", //                     2
    "", //                      3
    "pub @template ncp($n: ident) {", // 4
    "  model { }", //           5
    "}", //                     6
  ].join("\n");

  assert.equal(firstUnsupportedLine(text), 4);

  const real = at(2, "stanc");
  const bogus = at(4, "stanc");
  assert.deepEqual(filterDiagnostics([real, bogus], false, ctx(text)), [real]);
});

// --- line-anchored codes ------------------------------------------------

test("`forbidden-block` is hidden on a @macro header but not on a real block", () => {
  const text = [
    "pub @macro priors(each $p: ident) : stmt in model {", // 0 -- false positive
    "  $p ~ normal(0, 1);", //                                1
    "}", //                                                   2
    "", //                                                    3
    "model {", //                                             4 -- genuinely forbidden
    "}", //                                                   5
  ].join("\n");
  const c = ctx(text);

  assert.equal(isUnsupportedSyntaxNoise(at(0, "laplace", "forbidden-block"), c), true);
  assert.equal(
    isUnsupportedSyntaxNoise(at(4, "laplace", "forbidden-block"), c),
    false,
    "a real `model` block in a .laplacelib is still an error",
  );
});

test("`unknown-export` is hidden on @use/@expand but not on an ordinary call", () => {
  const text = [
    "@use stats::ncp(theta, K);", //                  0 -- a template, not an export
    "model {", //                                     1
    "  @expand stats::priors([a], normal(0, 1));", // 2 -- a macro, not an export
    "  y ~ normal(stats::typpo(x), 1);", //           3 -- a genuine typo
    "}", //                                           4
  ].join("\n");
  const c = ctx(text);

  assert.equal(isUnsupportedSyntaxNoise(at(0, "laplace", "unknown-export"), c), true);
  assert.equal(isUnsupportedSyntaxNoise(at(2, "laplace", "unknown-export"), c), true);
  assert.equal(
    isUnsupportedSyntaxNoise(at(3, "laplace", "unknown-export"), c),
    false,
    "a mistyped pkg::func must stay flagged",
  );
});

test("import and lockfile diagnostics are never hidden", () => {
  const text = "pub @template t($n: ident) { }";
  const c = ctx(text);
  for (const code of ["unresolved-import", "version-mismatch", "not-installed", "bad-package", "library-block", "codegen"]) {
    assert.equal(isUnsupportedSyntaxNoise(at(0, "laplace", code), c), false, `${code} must stay`);
  }
});

test("showing returns the very same array", () => {
  const diags = [at(0, "stanc")];
  assert.equal(filterDiagnostics(diags, true, ctx("pub @macro m($p: ident) : stmt in model {")), diags);
});

test("a code may arrive as a number or a {value} object", () => {
  const text = "@use stats::ncp(a);";
  const c = ctx(text);
  assert.equal(isUnsupportedSyntaxNoise({ ...at(0, "laplace"), code: { value: "unknown-export" } }, c), true);
  assert.equal(isUnsupportedSyntaxNoise({ ...at(0, "laplace"), code: 42 }, c), false);
});

test("an unreadable document hides nothing", () => {
  // What the extension passes when it cannot find the open document.
  const blind: DocumentContext = { firstUnsupportedLine: undefined, lineAt: () => undefined };
  const diags = [at(0, "stanc"), at(1, "laplace", "forbidden-block")];
  assert.deepEqual(filterDiagnostics(diags, false, blind), diags);
});

// --- detecting patch-1 syntax -------------------------------------------

test("each patch-1 construct is detected", () => {
  for (const line of [
    "pub real mean(vector x) {",
    "real f(real x, func(real) -> real g) {",
    "  matrix[num_elements(x), @wait(f).size] out;",
    "vector[2] to_pair(real x) {",
    "pub @template ncp($name: ident, $N: expr) {",
    "pub @macro priors(each $p: ident) : stmt in model {",
    "@use stats::ncp(theta, K);",
    "  @expand stats::priors([a, b], normal(0, 1));",
    "    vector[$N] ${name}_raw;",
  ]) {
    assert.equal(lineHasUnsupportedSyntax(line), true, `not detected: ${line}`);
  }
});

test("ordinary Stan is not mistaken for patch-1 syntax", () => {
  for (const line of [
    "real churn_frailty_ll(vector log_h, vector H, real theta, data vector d) {",
    "  vector[N] mu = X * beta;",
    "  matrix[N, 3] X;",
    "  vector[3] beta;",
    "  array[] real f(real x) {", // valid Stan return type, not a sized one
    "  for (n in 1:N) {",
    "  y ~ normal(mu, sigma);",
    "  real m = mean(x);",
    "model {",
    "  y_rep[n] = normal_rng(mu[n], sigma);",
  ]) {
    assert.equal(lineHasUnsupportedSyntax(line), false, `false positive: ${line}`);
  }
});

test("a comment mentioning a directive does not count", () => {
  assert.equal(lineHasUnsupportedSyntax("// use @template for this"), false);
  assert.equal(lineHasUnsupportedSyntax("// \\sum_{n} $x"), false);
  // ...but code before a trailing comment does.
  assert.equal(lineHasUnsupportedSyntax("pub real f(real x) { // a public one"), true);
});

/// The strongest guarantee available without an editor: the fixtures that
/// predate patch 1 must contain none of it, so opening them with the setting
/// off behaves exactly as it always did.
test("the pre-patch-1 fixtures contain no patch-1 syntax", () => {
  const fixtures = join(__dirname, "..", "..", "fixtures");
  for (const name of ["model.laplace", "stats.laplacelib", "wrapped.laplacelib"]) {
    const text = readFileSync(join(fixtures, name), "utf8");
    const line = firstUnsupportedLine(text);
    assert.equal(
      line,
      undefined,
      `${name} line ${(line ?? 0) + 1} looks like patch-1 syntax: ${text.split("\n")[line ?? 0]}`,
    );
  }
});

test("the patch-1 fixtures are detected, at their first construct", () => {
  const fixtures = join(__dirname, "..", "..", "fixtures", "patch1");
  for (const name of ["stats_patch1.laplacelib", "model_patch1.laplace"]) {
    const text = readFileSync(join(fixtures, name), "utf8");
    const line = firstUnsupportedLine(text);
    assert.notEqual(line, undefined, `${name}: no patch-1 syntax detected`);
    assert.equal(
      lineHasUnsupportedSyntax(text.split("\n")[line as number]),
      true,
      `${name}: reported line does not actually match`,
    );
  }
});
