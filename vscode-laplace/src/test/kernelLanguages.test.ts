// The sampler languages, `.lapkernel` and `.lapsampler`: registration and
// language configuration. Grammar output is covered by the snapshot tests
// in `fixtures/sampler/`.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const root = join(__dirname, "..", "..");
const read = (p: string) => JSON.parse(readFileSync(join(root, p), "utf8"));

interface Language {
  id: string;
  extensions: string[];
  configuration: string;
}
interface LanguageConfig {
  comments: { lineComment: string; blockComment?: [string, string] };
  brackets: [string, string][];
  autoClosingPairs: { open: string; close: string }[];
  wordPattern: string;
  folding?: { markers: { start: string; end: string } };
  onEnterRules?: unknown;
}

const pkg = read("package.json") as { contributes: { languages: Language[] } };
const lang = (id: string) => {
  const l = pkg.contributes.languages.find((x) => x.id === id);
  assert.ok(l, `language ${id} is not contributed`);
  return l;
};
const config = (id: string) => read(lang(id).configuration) as LanguageConfig;

for (const [id, ext] of [
  ["lapkernel", ".lapkernel"],
  ["lapsampler", ".lapsampler"],
] as const) {
  test(`${id} is registered for ${ext} files, with a configuration that exists`, () => {
    const l = lang(id);
    assert.deepEqual(l.extensions, [ext]);
    assert.ok(existsSync(join(root, l.configuration)), l.configuration);
  });

  test(`${id}: // and /* */ comments toggle (the grammar allows both)`, () => {
    const c = config(id);
    assert.equal(c.comments.lineComment, "//");
    assert.deepEqual(c.comments.blockComment, ["/*", "*/"]);
  });

  test(`${id}: braces are brackets, auto-close and fold`, () => {
    const c = config(id);
    assert.ok(c.brackets.some(([o, cl]) => o === "{" && cl === "}"));
    assert.ok(c.autoClosingPairs.some((p) => p.open === "{" && p.close === "}"));
    const { start, end } = c.folding!.markers;
    assert.match("  step {", new RegExp(start));
    assert.match("kernel mala {  // the kernel", new RegExp(start));
    assert.doesNotMatch("  step { move(q); }", new RegExp(start));
    assert.match("  }", new RegExp(end));
  });

  test(`${id}: no string auto-closing (the language has no strings)`, () => {
    assert.ok(!config(id).autoClosingPairs.some((p) => p.open === '"'));
  });

  test(`${id}: //@ lines do not continue on Enter, as in .laplace`, () => {
    assert.equal(config(id).onEnterRules, undefined);
    assert.equal(read("language-configuration.json").onEnterRules, undefined);
  });
}

const words = (id: string, line: string) =>
  line.match(new RegExp(config(id).wordPattern, "g")) ?? [];

test("lapkernel: @adapt is one word, so a snippet replaces the @ too", () => {
  assert.deepEqual(words("lapkernel", "real eps = 1 @adapt(step_size, target_accept = 0.574);"), [
    "real",
    "eps",
    "1",
    "@adapt",
    "step_size",
    "target_accept",
    "0.574",
  ]);
});

test("lapsampler: a namespaced kernel is one word", () => {
  assert.deepEqual(words("lapsampler", "block(mu, tau) as hyper : adapt(langevin::mala(eps = 0.05))"), [
    "block",
    "mu",
    "tau",
    "as",
    "hyper",
    "adapt",
    "langevin::mala",
    "eps",
    "0.05",
  ]);
});

test("the .laplace languages keep their own configuration", () => {
  assert.equal(lang("laplace").configuration, "./language-configuration.json");
  assert.equal(lang("laplacelib").configuration, "./language-configuration.json");
});
