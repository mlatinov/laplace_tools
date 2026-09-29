//! Turning the ```` ```math ```` fences `laplace-lsp` emits into rendered
//! formulas.
//!
//! `laplace-lsp` puts a function's `//@math` section in its own fenced block
//! with the language tag `math` (see `laplace-lsp/src/hover.rs`); everything
//! else stays in the plain fence it always used. Nothing in VS Code renders
//! such a fence, so without this it shows as a code block of raw LaTeX --
//! which is also exactly the fallback we want when rendering is off or the
//! TeX is broken.
//!
//! Deliberately free of any `vscode` import, so it can be unit-tested in
//! plain Node.

import { MathRenderError, Theme } from "./math";

/// A fenced block tagged `math`, capturing its body.
const MATH_FENCE = /^[ \t]*```math[ \t]*\r?\n([\s\S]*?)^[ \t]*```[ \t]*$/gm;

/// Whether `markdown` has anything for [`renderMathFences`] to do. Lets the
/// caller skip allocating a new hover for the common no-math case.
export function hasMathFence(markdown: string): boolean {
  MATH_FENCE.lastIndex = 0;
  return MATH_FENCE.test(markdown);
}

/// Replace every ```` ```math ```` fence with an image of the rendered
/// formula.
///
/// `render` is [`renderMath`](./math) in production and a stub in tests. If
/// it throws, that one fence degrades to a `latex` code block plus a short
/// note and the rest of the hover is untouched -- a formula that will not
/// compile must never cost the user the docs around it.
export function renderMathFences(
  markdown: string,
  theme: Theme,
  render: (tex: string, theme: Theme) => string,
): string {
  MATH_FENCE.lastIndex = 0;
  return markdown.replace(MATH_FENCE, (_whole, tex: string) => {
    const source = tex.replace(/\s+$/, "");
    try {
      const uri = render(source, theme);
      // A blank line on each side: an image directly against surrounding
      // text renders inline, and this is display math.
      return `\n![${altText(source)}](${uri})\n`;
    } catch (err) {
      const why = err instanceof MathRenderError ? `: ${err.message}` : "";
      return `\n_math could not be rendered${why}_\n\n\`\`\`latex\n${source}\n\`\`\`\n`;
    }
  });
}

/// Alt text for the image. Markdown link syntax breaks on an unescaped `]`
/// or newline, and LaTeX is full of both.
function altText(tex: string): string {
  const oneLine = tex.replace(/\s+/g, " ").trim();
  const clipped = oneLine.length > 80 ? `${oneLine.slice(0, 79)}…` : oneLine;
  return clipped.replace(/[[\]]/g, "");
}
