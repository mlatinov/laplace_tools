// The .lapkernel and .lapsampler snippets, expanded with their default
// placeholders:
//  - the two kernel snippets are byte for byte what `laplace new kernel`
//    writes, so the editor and the CLI scaffold the same file;
//  - every snippet passes `laplace sampler check` (`block` and `adapt` are
//    fragments, so they are checked inside the expanded `sampler`).
// Both need the `laplace` CLI (0.3.0 or later, with laplace-sample). Without
// it on PATH those tests are skipped with a message, unless
// LAPLACE_REQUIRE_CLI=1, which makes a missing CLI a failure (for CI).

import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const root = join(__dirname, "..", "..");
type Snippet = { prefix: string[]; body: string[] };
const load = (lang: string) =>
  JSON.parse(readFileSync(join(root, "snippets", `${lang}.json`), "utf8")) as Record<string, Snippet>;
const byPrefix = (lang: string, prefix: string) => {
  const s = Object.values(load(lang)).find((x) => x.prefix.includes(prefix));
  assert.ok(s, `no ${lang} snippet with prefix ${prefix}`);
  return s;
};

/// Expand a snippet body the way VS Code does when every placeholder keeps
/// its default: `${n:default}` becomes its default (and `$n` mirrors the
/// value of placeholder n), `${n|a,b|}` its first choice, `$n`/`${n}` with
/// no default and `$0` become empty, and `\$ \} \\` are literals.
export function expand(body: string[]): string {
  const text = body.join("\n");
  const defaults = new Map<string, string>();
  const placeholder = /\$\{(\d+):((?:\\.|[^}\\])*)\}/g;
  for (const m of text.matchAll(placeholder)) {
    if (!defaults.has(m[1])) defaults.set(m[1], m[2]);
  }
  return text
    .replace(placeholder, (_, _n, d: string) => d)
    .replace(/\$\{(\d+)\|([^,|]*)[^}]*\|\}/g, (_, _n, first: string) => first)
    .replace(/\$\{(\d+)\}|\$(\d+)/g, (_, a?: string, b?: string) => defaults.get(a ?? b ?? "") ?? "")
    .replace(/\\([$}\\])/g, "$1");
}

test("expand: defaults, mirrors, choices, empty tab stops and escapes", () => {
  assert.equal(expand(["a ${1:x} $1 ${1} ${2|p,q|} $3$0 \\$ \\}"]), "a x x x p  $ }");
});

const cli = spawnSync("laplace", ["--version"], { encoding: "utf8" });
const hasCli = cli.status === 0;
const skip = hasCli
  ? false
  : process.env.LAPLACE_REQUIRE_CLI === "1"
    ? false
    : "the `laplace` CLI is not on PATH (set LAPLACE_REQUIRE_CLI=1 to make this a failure)";

function check(file: string): void {
  const r = spawnSync("laplace", ["sampler", "check", file], { encoding: "utf8" });
  assert.equal(r.status, 0, `laplace sampler check ${file} failed:\n${r.stdout}${r.stderr}`);
}

for (const [prefix, kind] of [
  ["kernel", "proposal"],
  ["kernel-direct", "direct"],
] as const) {
  test(`lapkernel \`${prefix}\` is what \`laplace new kernel --kind ${kind}\` writes, and checks`, { skip }, () => {
    const expanded = expand(byPrefix("lapkernel", prefix).body);
    const name = expanded.match(/^kernel (\w+) \{$/m)?.[1];
    assert.ok(name, "the snippet names its kernel");
    const dir = mkdtempSync(join(tmpdir(), "laplace-snippet-"));
    execFileSync("laplace", ["new", "kernel", name, "--kind", kind], { cwd: dir, stdio: "pipe" });
    const scaffolded = readFileSync(join(dir, name, "kernels", `${name}.lapkernel`), "utf8");
    assert.equal(expanded, scaffolded);
    const file = join(dir, `${name}.lapkernel`);
    writeFileSync(file, expanded);
    check(file);
  });
}

test("lapsampler `sampler`, with `block` and `adapt` lines inside it, checks", { skip }, () => {
  const sampler = expand(byPrefix("lapsampler", "sampler").body);
  const block = expand(byPrefix("lapsampler", "block").body);
  const adapt = expand(byPrefix("lapsampler", "adapt").body);
  assert.match(sampler, /^ {4}rest : nuts\(\)$/m);
  const dir = mkdtempSync(join(tmpdir(), "laplace-snippet-"));

  const alone = join(dir, "alone.lapsampler");
  writeFileSync(alone, sampler);
  check(alone);

  const lines = `    ${block}\n    block(other) : ${adapt}\n`;
  const combined = join(dir, "combined.lapsampler");
  writeFileSync(combined, sampler.replace(/^ {4}rest : nuts\(\)$/m, `${lines}    rest : nuts()`));
  check(combined);
});
