import assert from "node:assert/strict";
import { test } from "node:test";

import { CodedDiagnostic, filterDiagnostics, isUnsupportedSyntaxNoise } from "../diagnosticFilter";

const laplace = (code: string): CodedDiagnostic => ({ source: "laplace", code });
const stanc = (code?: string): CodedDiagnostic => ({ source: "stanc", code });

test("the two codes patch-1 syntax provokes are suppressible", () => {
  assert.equal(isUnsupportedSyntaxNoise(laplace("forbidden-block")), true);
  assert.equal(isUnsupportedSyntaxNoise(laplace("unknown-export")), true);
});

test("import and lockfile diagnostics stay visible", () => {
  // These are true whatever the file's syntax looks like, so hiding them
  // would cost the user a real error.
  for (const code of ["unresolved-import", "version-mismatch", "not-installed", "bad-package", "library-block", "codegen"]) {
    assert.equal(isUnsupportedSyntaxNoise(laplace(code)), false, `${code} should stay visible`);
  }
});

test("everything from the stanc pass is suppressible", () => {
  assert.equal(isUnsupportedSyntaxNoise(stanc()), true);
  assert.equal(isUnsupportedSyntaxNoise(stanc("anything")), true);
});

test("a diagnostic from somewhere else with no code is left alone", () => {
  assert.equal(isUnsupportedSyntaxNoise({}), false);
  assert.equal(isUnsupportedSyntaxNoise({ source: "laplace" }), false);
});

test("a code can arrive as a number or as a {value, target} object", () => {
  assert.equal(isUnsupportedSyntaxNoise({ code: { value: "unknown-export" } }), true);
  assert.equal(isUnsupportedSyntaxNoise({ code: { value: "unresolved-import" } }), false);
  assert.equal(isUnsupportedSyntaxNoise({ code: 42 }), false);
});

test("showing diagnostics returns the very same array", () => {
  const diags = [laplace("forbidden-block"), stanc()];
  assert.equal(filterDiagnostics(diags, true), diags, "the normal path should not allocate");
});

test("hiding keeps the real errors and drops the noise", () => {
  const diags = [
    laplace("unresolved-import"),
    laplace("forbidden-block"),
    stanc("syntax"),
    laplace("version-mismatch"),
    laplace("unknown-export"),
  ];

  const kept = filterDiagnostics(diags, false);
  assert.deepEqual(
    kept.map((d) => d.code),
    ["unresolved-import", "version-mismatch"],
  );
});
