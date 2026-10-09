import assert from "node:assert/strict";
import { test } from "node:test";

import { COMMANDS, completeAt, findSymbols, greekName } from "../symbols";

/// The spans in `text`, as `[covered text, drawn as]` pairs.
function drawn(text: string): [string, string][] {
  return findSymbols(text).map((s) => [
    text.slice(s.start, s.end),
    s.subscript ? `${s.symbol}_${s.subscript}` : s.symbol,
  ]);
}

test("a Greek-named variable is drawn as its letter", () => {
  assert.deepEqual(drawn("  theta ~ normal(mu, sigma);"), [
    ["theta", "θ"],
    ["mu", "μ"],
    ["sigma", "σ"],
  ]);
});

test("everything after the first underscore is the subscript", () => {
  assert.deepEqual(greekName("sigma_obs"), { symbol: "σ", subscript: "obs" });
  assert.deepEqual(greekName("theta_raw"), { symbol: "θ", subscript: "raw" });
  assert.deepEqual(greekName("sigma_y_rep"), { symbol: "σ", subscript: "y_rep" });
});

test("only whole names count: `eta` inside `theta` or `beta_hat_x` style prefixes", () => {
  assert.equal(greekName("theta2"), undefined);
  assert.equal(greekName("betas"), undefined);
  assert.equal(greekName("log_sigma"), undefined);
  assert.equal(greekName("sigma_"), undefined);
  assert.deepEqual(drawn("real theta;"), [["theta", "θ"]]);
});

test("upper-case letters and the LaTeX variants", () => {
  assert.deepEqual(drawn("matrix[K, K] Sigma;"), [["Sigma", "Σ"]]);
  assert.deepEqual(drawn("real varepsilon; real epsilon;"), [
    ["varepsilon", "ε"],
    ["epsilon", "ϵ"],
  ]);
});

test("Stan's beta() and gamma() keep their names: Greek names are never drawn at a call", () => {
  assert.deepEqual(drawn("  y ~ beta(a, b);\n  z ~ gamma(alpha, beta);"), [
    ["alpha", "α"],
    ["beta", "β"],
  ]);
});

test("sum, prod and sqrt are drawn at a call, and keep their parentheses", () => {
  assert.deepEqual(drawn("real s = sum(x) + prod(y) * sqrt(z) + pi();"), [
    ["sum", "∑"],
    ["prod", "∏"],
    ["sqrt", "√"],
    ["pi", "π"],
  ]);
  // Not a call: a variable that happens to be called `sum` stays as it is.
  assert.deepEqual(drawn("real sum;"), []);
});

test("comparison operators", () => {
  assert.deepEqual(drawn("if (a <= b && c >= d && e != f) {}"), [
    ["<=", "≤"],
    [">=", "≥"],
    ["!=", "≠"],
  ]);
  // `<lower=0>` holds no `<=`.
  assert.deepEqual(drawn("real<lower=0> x;"), []);
});

test("nothing is drawn in comments or strings", () => {
  const text = [
    "// sigma is the noise scale",
    "//@math \\sigma \\leq 1",
    "/* mu <= 0 */",
    'print("sigma = ", x);',
  ].join("\n");
  assert.deepEqual(drawn(text), []);
});

test("namespaced names, accessors and placeholders are left alone", () => {
  assert.deepEqual(drawn("real y = pkg::sigma(x) + pkg::sum(x);"), []);
  assert.deepEqual(drawn("sigma::f(x);"), []);
  assert.deepEqual(drawn("matrix[n, @wait(f).size] out;"), []);
  assert.deepEqual(drawn("vector[$N] ${sigma}_raw; $mu ~ normal(0, 1);"), []);
});

// --- input -----------------------------------------------------------------

test("typing `\\` offers every command, replacing from the backslash", () => {
  const result = completeAt("  real \\si", "");
  assert.ok(result);
  assert.equal(result.start, 7);
  assert.equal(result.end, 10);
  const sigma = result.items.find((i) => i.label === "\\sigma");
  assert.deepEqual(sigma, { label: "\\sigma", symbol: "σ", insert: "sigma" });
});

test("a braced subscript completes to one name, LaTeX-style", () => {
  const result = completeAt("  real \\sigma_{obs}", ";");
  assert.ok(result);
  assert.deepEqual(result.items, [{ label: "\\sigma_{obs}", symbol: "σ_obs", insert: "sigma_obs" }]);
  assert.equal(result.end, "  real \\sigma_{obs}".length);
});

test("an auto-closed brace after the cursor is replaced too", () => {
  const before = "  real \\sigma_{obs";
  const result = completeAt(before, "};");
  assert.ok(result);
  assert.equal(result.items[0].insert, "sigma_obs");
  assert.equal(result.end, before.length + 1);
});

test("a bare subscript works the same", () => {
  const result = completeAt("\\mu_y", "");
  assert.ok(result);
  assert.deepEqual(result.items, [{ label: "\\mu_y", symbol: "μ_y", insert: "mu_y" }]);
});

test("a subscript on something that is not a Greek letter offers nothing", () => {
  assert.equal(completeAt("\\foo_x", ""), undefined);
  assert.equal(completeAt("\\sigma_", ""), undefined);
  assert.equal(completeAt("real x", ""), undefined);
});

test("operator commands insert the Stan spelling", () => {
  const insert = (label: string) => COMMANDS.find((c) => c.label === label)?.insert;
  assert.equal(insert("\\leq"), "<=");
  assert.equal(insert("\\ge"), ">=");
  assert.equal(insert("\\neq"), "!=");
  assert.equal(insert("\\sum"), "sum");
  assert.equal(insert("\\pi"), "pi()");
});

test("everything the input inserts is drawn back as the symbol it promised", () => {
  for (const c of COMMANDS) {
    if (c.insert.startsWith("<") || c.insert.startsWith(">") || c.insert.startsWith("!")) {
      assert.deepEqual(drawn(`a ${c.insert} b`), [[c.insert, c.symbol]], c.label);
    } else if (["sum", "prod", "sqrt"].includes(c.insert)) {
      assert.deepEqual(drawn(`${c.insert}(x)`), [[c.insert, c.symbol]], c.label);
    } else {
      // A variable, or `pi()`.
      assert.deepEqual(drawn(c.insert), [[c.insert.replace("()", ""), c.symbol]], c.label);
    }
  }
});
