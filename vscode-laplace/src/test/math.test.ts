import assert from "node:assert/strict";
import { test } from "node:test";

import { clearCache, MathRenderError, renderMath } from "../math";

/// The three-line `//@math` block from a real library, the hardest case the
/// spec names: `\\` breaks, `\operatorname`, nested `\left...\right`, and a
/// very long final line.
const CHURN_CURE = String.raw`\pi_n = \operatorname{logit}^{-1}(\psi_n) \\
\log h(t_n) = \log\beta - \log\alpha_n + (\beta - 1)\left(\log t_n - \log\alpha_n\right) - \operatorname{log1p}\left((t_n/\alpha_n)^{\beta}\right), \quad H(t_n) = \operatorname{log1p}\left((t_n/\alpha_n)^{\beta}\right) \\
\log L = \sum_{n=1}^{N} \left\{ d_n\left[\log(1-\pi_n) + \log h(t_n) - H(t_n)\right] + (1-d_n)\log\left(\pi_n + (1-\pi_n)\,e^{-H(t_n)}\right) \right\}`;

function decode(uri: string): string {
  const prefix = "data:image/svg+xml;base64,";
  assert.ok(uri.startsWith(prefix), `not an svg data uri: ${uri.slice(0, 60)}`);
  return Buffer.from(uri.slice(prefix.length), "base64").toString("utf8");
}

test("the churn-cure formula renders to a standalone svg", () => {
  clearCache();
  const svg = decode(renderMath(CHURN_CURE, "dark"));

  assert.match(svg, /^<svg/, "should be an svg element");
  assert.match(svg, /xmlns="http:\/\/www\.w3\.org\/2000\/svg"/, "needs its own namespace to load as an image");
  // `fontCache: "local"` must keep the glyph paths inside this svg; with the
  // global cache they end up in a shared <defs> the image cannot reach.
  assert.match(svg, /<path/, "no glyph paths -- font cache is not local");
  assert.doesNotMatch(svg, /Undefined control sequence/);
  // Three `\\`-separated rows inside a `gathered`, so the breaks took.
  assert.ok(svg.length > 20000, `suspiciously small for three formulas: ${svg.length} bytes`);
});

test("the section is treated as display math, so `\\\\` breaks lines", () => {
  clearCache();
  const oneLine = decode(renderMath(String.raw`a = b`, "dark"));
  const twoLines = decode(renderMath(String.raw`a = b \\ c = d`, "dark"));

  const heightOf = (svg: string) => Number(/height="([\d.]+)px"/.exec(svg)?.[1] ?? 0);
  assert.ok(
    heightOf(twoLines) > heightOf(oneLine) * 1.5,
    `two rows should be much taller: ${heightOf(oneLine)} vs ${heightOf(twoLines)}`,
  );
});

test("the theme foreground is baked into the svg", () => {
  clearCache();
  const light = decode(renderMath(String.raw`x^2`, "light"));
  const dark = decode(renderMath(String.raw`x^2`, "dark"));

  assert.match(light, /color:#1f1f1f/);
  assert.match(dark, /color:#cccccc/);
  assert.notEqual(light, dark, "a theme switch must change the image");
});

test("a long formula is capped in width but not shrunk to mush", () => {
  clearCache();
  const svg = decode(renderMath(CHURN_CURE, "dark"));
  const width = Number(/width="([\d.]+)px"/.exec(svg)?.[1] ?? 0);

  assert.ok(width > 0, "no pixel width on the svg");
  assert.ok(width <= 960, `wider than the cap: ${width}`);
  // MIN_SCALE keeps it legible rather than fitting the cap exactly.
  assert.ok(width > 400, `shrunk past legibility: ${width}`);
});

test("invalid TeX throws rather than rendering a picture of an error", () => {
  clearCache();
  assert.throws(
    () => renderMath(String.raw`\left( x`, "dark"),
    MathRenderError,
    "an unbalanced \\left( should not come back as an image",
  );
  assert.throws(() => renderMath(String.raw`\nosuchmacro{x}`, "dark"), MathRenderError);
});

test("rendering the same formula twice is served from the cache", () => {
  clearCache();
  const first = renderMath(CHURN_CURE, "dark");
  const second = renderMath(CHURN_CURE, "dark");
  assert.equal(first, second);
  // Same source, other theme: a different image, so the key includes both.
  assert.notEqual(renderMath(CHURN_CURE, "light"), first);
});

/// MathJax numbers each conversion's glyph definitions (`MJX-1-TEX-...`,
/// `MJX-2-TEX-...`), so two independent renders of identical TeX are never
/// byte-equal. Normalise that counter away to compare the actual drawing.
function normalizeGlyphIds(svg: string): string {
  return svg.replace(/MJX-\d+-/g, "MJX-");
}

test("a leftover `// ` comment marker is stripped before rendering", () => {
  clearCache();
  const bare = decode(renderMath(String.raw`\alpha + \beta`, "dark"));
  clearCache();
  const commented = decode(renderMath("// \\alpha + \\beta", "dark"));
  assert.equal(normalizeGlyphIds(commented), normalizeGlyphIds(bare));
});
