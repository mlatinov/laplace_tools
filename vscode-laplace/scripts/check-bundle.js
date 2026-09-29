// Checks the built `out/extension.js` for the two faults that only exist in
// the bundle, and so cannot be caught by tests that import from `src/`.
//
// One of them was real. `mathjax-full/js/components/version.js` reads its own
// version through `eval('require')(path.resolve(__dirname, '..', '..',
// 'package.json'))`, deliberately hidden from bundlers, which are expected to
// supply a `PACKAGE_VERSION` global instead. Without that define the eval'd
// require survives into the bundle and resolves relative to the *output*
// file, so the packaged extension threw MODULE_NOT_FOUND the moment a hover
// touched MathJax -- while every unit test passed, because `out/test/`
// happens to sit one directory deeper than `out/` and so resolved to the
// extension's own package.json by luck.
//
// This is a static check. It does not prove the extension activates; that
// needs a real editor (`@vscode/test-electron`), which this package does not
// set up.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { builtinModules } = require("node:module");

const BUNDLE = path.resolve(__dirname, "..", "out", "extension.js");

assert.ok(fs.existsSync(BUNDLE), `no bundle at ${BUNDLE} -- run \`npm run compile\` first`);
const source = fs.readFileSync(BUNDLE, "utf8");

// 1. The define must have been applied. esbuild substitutes the literal for
//    every `PACKAGE_VERSION`, so the identifier surviving anywhere means it
//    was not defined and the eval'd require is live.
assert.ok(
  !source.includes("PACKAGE_VERSION"),
  "PACKAGE_VERSION was not substituted -- is it still defined in esbuild.js? " +
    "Without it, mathjax-full's eval'd require runs and throws MODULE_NOT_FOUND.",
);

// 2. ...and with it defined, the guard folds to `false`, so any surviving
//    `eval("require")` sits in a provably dead branch. `--production`
//    (minify) drops the branch outright; a dev build keeps the text, which is
//    harmless but must still be unreachable.
for (const match of source.matchAll(/eval\((["'])require\1\)/g)) {
  const before = source.slice(Math.max(0, match.index - 120), match.index);
  assert.match(
    before,
    /(false|!1)\s*\?/,
    "an eval'd require is reachable in the bundle -- it would resolve relative to out/ and throw",
  );
}

// 3. Nothing may be resolved from `node_modules` at runtime, because the VSIX
//    is packaged with `--no-dependencies` and ships none. Commit 30b24e3
//    fixed the opposite failure (a VSIX built with `--no-dependencies` that
//    was *not* bundled, so `vscode-languageclient` was missing and activation
//    threw); this is the check that keeps that fix from regressing now that
//    bundling is what makes `--no-dependencies` correct.
const allowed = new Set(["vscode", ...builtinModules, ...builtinModules.map((m) => `node:${m}`)]);
const required = [...source.matchAll(/require\("([^"]+)"\)/g)].map((m) => m[1]);
const unexpected = [...new Set(required.filter((r) => !allowed.has(r)))];
assert.deepEqual(
  unexpected,
  [],
  `bundle requires these at runtime, but the VSIX ships no node_modules: ${unexpected.join(", ")}`,
);

const builtinsUsed = [...new Set(required)].filter((r) => r !== "vscode").sort();
console.log(
  `check-bundle: ${(source.length / 1024 / 1024).toFixed(2)}MB, requires only vscode + ${builtinsUsed.length} builtins (${builtinsUsed.join(", ")})`,
);
