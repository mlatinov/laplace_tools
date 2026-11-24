// The sampler languages, `.lapkernel` and `.lapsampler`: registration and
// language configuration. Grammar output is covered by the snapshot tests
// in `fixtures/sampler/`.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { SERVER_LANGUAGES } from "../languages";

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

// TextMate silently ignores an include that points nowhere, so a renamed
// rule in the .laplace grammar would quietly drop scopes from these two.
type Rule = { include?: string; patterns?: Rule[]; [k: string]: unknown };
const grammars = (
  read("package.json") as {
    contributes: { grammars: { language: string; scopeName: string; path: string }[] };
  }
).contributes.grammars;

function includes(rule: unknown, out: string[] = []): string[] {
  if (Array.isArray(rule)) rule.forEach((r) => includes(r, out));
  else if (rule && typeof rule === "object") {
    for (const [k, v] of Object.entries(rule)) {
      if (k === "include" && typeof v === "string") out.push(v);
      else includes(v, out);
    }
  }
  return out;
}

for (const id of ["lapkernel", "lapsampler"]) {
  test(`${id}: the grammar is registered and every include resolves`, () => {
    const g = grammars.find((x) => x.language === id);
    assert.ok(g, `no grammar for ${id}`);
    assert.equal(g.scopeName, `source.${id}`);
    const grammar = read(g.path) as { scopeName: string; repository: Record<string, Rule> };
    assert.equal(grammar.scopeName, g.scopeName);
    for (const inc of includes(grammar)) {
      if (inc === "$self" || inc === "$base") continue;
      const [scope, key] = inc.startsWith("#") ? [g.scopeName, inc.slice(1)] : inc.split("#");
      const target = grammars.find((x) => x.scopeName === scope);
      assert.ok(target, `${inc}: no grammar has scope ${scope}`);
      const repo = (read(target.path) as { repository: Record<string, Rule> }).repository;
      assert.ok(key && repo[key], `${inc}: ${scope} has no rule \`${key}\``);
    }
  });
}

test("the server gets every contributed language, and each activates the extension", () => {
  const manifest = read("package.json") as {
    activationEvents: string[];
    contributes: { languages: Language[] };
  };
  const contributed = manifest.contributes.languages.map((l) => l.id).sort();
  assert.deepEqual([...SERVER_LANGUAGES].sort(), contributed);
  assert.deepEqual(
    manifest.activationEvents.sort(),
    SERVER_LANGUAGES.map((id) => `onLanguage:${id}`).sort(),
  );
});
