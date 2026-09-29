import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const config = JSON.parse(
  readFileSync(join(__dirname, "..", "..", "language-configuration.json"), "utf8"),
) as { wordPattern: string };

/// What VS Code uses `wordPattern` for, among other things: deciding which
/// span a completion replaces. A directive whose `@` is not part of the word
/// leaves the `@` behind when a snippet is accepted, which is how
/// `@macro` + the `macro` snippet produced `@pub @macro`.
function wordsIn(line: string): string[] {
  return line.match(new RegExp(config.wordPattern, "g")) ?? [];
}

test("a directive is one word, @ included", () => {
  // The bug this guards: `@` outside the word means a snippet accepted at
  // `@macro` replaces only `macro`.
  for (const directive of ["@template", "@macro", "@use", "@expand", "@wait"]) {
    assert.deepEqual(wordsIn(directive), [directive], `${directive} should be a single word`);
  }
});

test("a placeholder is one word in both forms", () => {
  assert.deepEqual(wordsIn("$name"), ["$name"]);
  assert.deepEqual(wordsIn("${name}"), ["${name}"]);
});

test("`${name}_raw` splits into the placeholder and the suffix", () => {
  // Matching the highlighting: `${name}` is the placeholder, `_raw` is an
  // ordinary identifier, so double-clicking either selects just that part.
  assert.deepEqual(wordsIn("${name}_raw"), ["${name}", "_raw"]);
});

test("a namespaced call is still one word", () => {
  // Pre-existing behaviour that must not regress.
  assert.deepEqual(wordsIn("stats::ncp"), ["stats::ncp"]);
});

test("ordinary identifiers and types are unaffected", () => {
  assert.deepEqual(wordsIn("vector x = mean(y);"), ["vector", "x", "mean", "y"]);
});

test("a real macro header tokenises as expected", () => {
  assert.deepEqual(wordsIn("pub @macro priors(each $p: ident, $dist: expr) : stmt in model {"), [
    "pub",
    "@macro",
    "priors",
    "each",
    "$p",
    "ident",
    "$dist",
    "expr",
    "stmt",
    "in",
    "model",
  ]);
});

test("every snippet that inserts a directive is reachable by typing it with the @", () => {
  for (const [file, expected] of [
    ["laplace.json", ["@use", "@expand"]],
    ["laplacelib.json", ["@template", "@macro"]],
  ] as [string, string[]][]) {
    const snippets = JSON.parse(
      readFileSync(join(__dirname, "..", "..", "snippets", file), "utf8"),
    ) as Record<string, { prefix: string | string[] }>;
    const prefixes = Object.values(snippets).flatMap((s) =>
      Array.isArray(s.prefix) ? s.prefix : [s.prefix],
    );
    for (const want of expected) {
      assert.ok(prefixes.includes(want), `${file} has no snippet with prefix ${want}`);
      // ...and the bare form still works, for anyone who types it that way.
      assert.ok(prefixes.includes(want.slice(1)), `${file} has no snippet with prefix ${want.slice(1)}`);
    }
  }
});
