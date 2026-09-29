import assert from "node:assert/strict";
import { test } from "node:test";

import { hasMathFence, renderMathFences } from "../docs";
import { MathRenderError, Theme } from "../math";

/// What `laplace-lsp`'s `hover::markdown` produces for a documented function
/// with a `//@math` section: the plain fence it always used, then the math in
/// its own tagged fence.
const HOVER = [
  "```",
  "gps::rbf_cov(x: vector) -> matrix",
  "",
  "Radial basis covariance.",
  "",
  "Parameters:",
  "  x  the inputs",
  "```",
  "",
  "```math",
  String.raw`\pi_n = \operatorname{logit}^{-1}(\psi_n) \\`,
  String.raw`\log L = \sum_n d_n`,
  "```",
  "",
].join("\n");

const stub = (tex: string, theme: Theme) => `data:image/svg+xml;base64,${theme}:${tex.length}`;

test("a math fence becomes an image and the rest is untouched", () => {
  const out = renderMathFences(HOVER, "dark", stub);

  assert.match(out, /!\[.*\]\(data:image\/svg\+xml;base64,dark:\d+\)/);
  assert.doesNotMatch(out, /```math/, "the fence should be gone");
  // The LaTeX survives only as the image's alt text -- which is what a
  // screen reader reads and what a copy of the hover keeps -- never as
  // visible raw source.
  assert.doesNotMatch(out, /^\\pi_n/m, "raw LaTeX should not be shown as text");
  const alt = /!\[([^\]]*)\]/.exec(out)?.[1];
  assert.match(alt ?? "", /\\pi_n/, "the source belongs in the alt text");
  // Every other section survives, still inside its original fence.
  assert.match(out, /gps::rbf_cov\(x: vector\) -> matrix/);
  assert.match(out, /Radial basis covariance\./);
  assert.match(out, /Parameters:/);
  assert.match(out, / {2}x {2}the inputs/);
});

test("both lines of the section go to one render call", () => {
  const seen: string[] = [];
  renderMathFences(HOVER, "dark", (tex) => {
    seen.push(tex);
    return "data:x";
  });

  assert.equal(seen.length, 1, "a section is one formula, not one per line");
  assert.match(seen[0], /\\pi_n/);
  assert.match(seen[0], /\\log L/);
  assert.match(seen[0], /\\\\\n/, "the `\\\\` line break must reach the renderer");
});

test("a hover with no math is returned unchanged", () => {
  const plain = "```\ngps::f(x: real) -> real\n\nNo formula here.\n```\n";
  assert.equal(hasMathFence(plain), false);
  assert.equal(
    renderMathFences(plain, "dark", () => {
      throw new Error("should not be called");
    }),
    plain,
  );
});

test("a failed render falls back to the source and keeps the hover", () => {
  const out = renderMathFences(HOVER, "dark", () => {
    throw new MathRenderError("Missing close brace");
  });

  assert.match(out, /math could not be rendered: Missing close brace/);
  assert.match(out, /```latex/, "the source should come back as a code block");
  assert.match(out, /\\operatorname/, "the LaTeX itself must not be dropped");
  // The docs around it survive -- a broken formula costs nothing else.
  assert.match(out, /Radial basis covariance\./);
  assert.match(out, /Parameters:/);
});

test("each of several fences is rendered independently", () => {
  const two = "```math\na = b\n```\n\ntext between\n\n```math\nc = d\n```\n";
  assert.equal(hasMathFence(two), true);

  const out = renderMathFences(two, "light", stub);
  assert.equal(out.match(/!\[/g)?.length, 2);
  assert.match(out, /text between/);
});

test("one broken fence does not stop the others rendering", () => {
  const two = "```math\nbad\n```\n\n```math\ngood\n```\n";
  const out = renderMathFences(two, "dark", (tex) => {
    if (tex === "bad") {
      throw new MathRenderError("nope");
    }
    return "data:ok";
  });

  assert.match(out, /math could not be rendered: nope/);
  assert.match(out, /!\[good\]\(data:ok\)/);
});

test("alt text is one line and free of brackets that would break the link", () => {
  const awkward = "```math\n\\left[ a \\right] \\\\\n b\n```\n";
  const out = renderMathFences(awkward, "dark", () => "data:x");

  const alt = /!\[([^\]]*)\]/.exec(out)?.[1];
  assert.ok(alt !== undefined, `no alt text in ${out}`);
  assert.doesNotMatch(alt, /[[\]\n]/, "alt text must not contain brackets or newlines");
});

test("hasMathFence is not left stateful between calls", () => {
  const withMath = "```math\na\n```\n";
  // A global regex with `g` keeps `lastIndex`; calling twice must agree.
  assert.equal(hasMathFence(withMath), true);
  assert.equal(hasMathFence(withMath), true);
});
