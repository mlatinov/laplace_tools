//! TeX -> SVG rendering for `//@math` doc sections, via MathJax running in
//! the extension host. No network access and no CDN: `mathjax-full` is
//! bundled into `out/extension.js` by esbuild.
//!
//! VS Code's hover and completion markdown does not render LaTeX, so the
//! only way to show a real formula there is to render it to an image and
//! embed it. A `data:` URI keeps that self-contained -- nothing is written
//! to disk and no webview is involved.

import { liteAdaptor, LiteAdaptor } from "mathjax-full/js/adaptors/liteAdaptor";
import { RegisterHTMLHandler } from "mathjax-full/js/handlers/html";
import { TeX } from "mathjax-full/js/input/tex";
import { AllPackages } from "mathjax-full/js/input/tex/AllPackages";
import { mathjax } from "mathjax-full/js/mathjax";
import { SVG } from "mathjax-full/js/output/svg";
import type { LiteElement } from "mathjax-full/js/adaptors/lite/Element";
import type { MathDocument } from "mathjax-full/js/core/MathDocument";

/// Which foreground a formula is drawn in. A `data:` URI image cannot
/// follow the theme through CSS -- `currentColor` in a standalone SVG
/// resolves against the SVG's own `color`, not the page's -- so the colour
/// is baked in at render time and the cache is keyed by it.
export type Theme = "light" | "dark";

/// Rendered at this many pixels per `em`, roughly an editor font size, so
/// the formula reads at the same weight as the code around it.
const EM_PX = 16;

/// `ex` is about half an `em` in the fonts MathJax ships.
const EX_PX = EM_PX / 2;

/// How wide a formula may get before it is scaled down. A VS Code hover is
/// narrower than this, and scales the image to fit; rendering wider than the
/// hover and letting it shrink is what keeps a long line legible instead of
/// clipped.
const MAX_WIDTH_PX = 960;

/// ...but never shrink past this, or a very long line (the `\log L` sum in
/// the churn-cure example) turns into grey mush. Past this point the hover
/// clips rather than shrinks, which at least leaves the visible part
/// readable.
const MIN_SCALE = 0.6;

/// The TeX extensions a doc comment may use: everything `AllPackages`
/// registers -- `ams` for `\begin{gathered}` and `\operatorname`,
/// `newcommand`, `mathtools`, ... -- minus four.
///
/// `require` and `autoload` need MathJax's lazy component loader, which the
/// direct API does not set up.
///
/// `noerrors` and `noundefined` are dropped deliberately: they exist to make
/// broken TeX render anyway, `noerrors` by drawing the original source and
/// `noundefined` by drawing an unknown macro's name in red. That is the
/// opposite of what is wanted here -- an error has to reach [`renderMath`] so
/// it can fall back to showing the LaTeX as text, rather than embedding a
/// picture of an error message in the hover.
const PACKAGES = AllPackages.filter(
  (p) => !["require", "autoload", "noerrors", "noundefined"].includes(p),
);

interface Pipeline {
  adaptor: LiteAdaptor;
  document: MathDocument<LiteElement, unknown, unknown>;
}

let pipeline: Pipeline | undefined;

/// Built on first use: registering the HTML handler is a global side effect
/// on the MathJax singleton, so it must happen exactly once per host.
function getPipeline(): Pipeline {
  if (pipeline) {
    return pipeline;
  }
  const adaptor = liteAdaptor();
  RegisterHTMLHandler(adaptor);
  const document = mathjax.document("", {
    InputJax: new TeX({ packages: PACKAGES }),
    // `fontCache: "local"` puts each formula's glyph definitions inside its
    // own `<svg>`. The default, `"global"`, hoists them into a shared
    // `<defs>` that a standalone image has no access to, which renders as
    // an empty box.
    OutputJax: new SVG({ fontCache: "local" }),
  }) as unknown as MathDocument<LiteElement, unknown, unknown>;
  pipeline = { adaptor, document };
  return pipeline;
}

/// `data:` URIs, keyed by theme and TeX source, so re-hovering the same
/// function is instant. Rendering is a few milliseconds but happens on the
/// UI-blocking hover path.
const cache = new Map<string, string>();

/// Drop everything cached. Called when the active theme changes: entries are
/// keyed by theme, so stale ones are already unreachable -- this just stops
/// them accumulating across a session.
export function clearCache(): void {
  cache.clear();
}

/// The foreground a formula is drawn in, per theme. Deliberately not the
/// exact `editor.foreground` (which the extension host cannot read for an
/// arbitrary theme) but the colour VS Code's own default themes use, which
/// reads correctly on either background.
function foreground(theme: Theme): string {
  return theme === "light" ? "#1f1f1f" : "#cccccc";
}

/// Strip the comment marker `laplace`'s doc parser may have left on each
/// line. It normally strips `// ` itself, so this is belt and braces for a
/// package whose `docs.json` was written by an older extractor.
function stripCommentMarkers(tex: string): string {
  return tex
    .split("\n")
    .map((line) => line.replace(/^\s*\/\/ ?/, ""))
    .join("\n");
}

/// A `//@math` section is display math, and its `\\` line breaks only mean
/// anything inside an environment that allows them.
function asDisplayMath(tex: string): string {
  return `\\begin{gathered}\n${tex.trim()}\n\\end{gathered}`;
}

/// `"23.485ex"` -> `23.485`. MathJax sizes its SVG in `ex`, which means
/// nothing to an `<img>`: there is no font context to resolve it against,
/// so it has to become pixels.
function exToPx(value: string | undefined): number | undefined {
  if (!value) {
    return undefined;
  }
  const match = /^(-?[\d.]+)ex$/.exec(value.trim());
  return match ? Number(match[1]) * EX_PX : undefined;
}

export class MathRenderError extends Error {}

/// Render one `//@math` section to a `data:image/svg+xml` URI.
///
/// Throws [`MathRenderError`] on invalid TeX so the caller can fall back to
/// showing the source; it never returns a broken image.
export function renderMath(tex: string, theme: Theme): string {
  const key = `${theme}\u0000${tex}`;
  const hit = cache.get(key);
  if (hit !== undefined) {
    return hit;
  }

  const { adaptor, document } = getPipeline();
  const source = asDisplayMath(stripCommentMarkers(tex));

  let svg: LiteElement;
  try {
    const container = document.convert(source, {
      display: true,
      em: EM_PX,
      ex: EX_PX,
      containerWidth: MAX_WIDTH_PX,
    }) as LiteElement;
    const first = adaptor.firstChild(container) as LiteElement | null;
    if (!first) {
      throw new Error("MathJax produced no SVG");
    }
    svg = first;
  } catch (err) {
    throw new MathRenderError(err instanceof Error ? err.message : String(err));
  }

  // MathJax does not throw on bad TeX: it renders the error, as an `merror`
  // MathML node, and hands back a perfectly valid SVG of the message. Look
  // for that node rather than for known message prefixes -- the set of
  // messages is long and version-dependent, the node is neither.
  if (adaptor.outerHTML(svg).includes('data-mml-node="merror"')) {
    throw new MathRenderError(adaptor.textContent(svg).trim() || "invalid TeX");
  }

  let width = exToPx(adaptor.getAttribute(svg, "width"));
  let height = exToPx(adaptor.getAttribute(svg, "height"));
  if (width !== undefined && height !== undefined && width > MAX_WIDTH_PX) {
    const scale = Math.max(MAX_WIDTH_PX / width, MIN_SCALE);
    width *= scale;
    height *= scale;
  }

  // A standalone SVG needs its own namespace to load as an image, and its
  // own `color` for MathJax's `currentColor` fills to resolve against.
  adaptor.setAttribute(svg, "xmlns", "http://www.w3.org/2000/svg");
  adaptor.setAttribute(svg, "style", `color:${foreground(theme)}`);
  if (width !== undefined && height !== undefined) {
    adaptor.setAttribute(svg, "width", `${width.toFixed(2)}px`);
    adaptor.setAttribute(svg, "height", `${height.toFixed(2)}px`);
  }

  const markup = adaptor.outerHTML(svg);
  const uri = `data:image/svg+xml;base64,${Buffer.from(markup, "utf8").toString("base64")}`;
  cache.set(key, uri);
  return uri;
}
