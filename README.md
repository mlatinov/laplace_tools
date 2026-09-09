# laplace-tools

Editor tooling for [`laplace`](../laplace) — a source-to-source preprocessor
that compiles `.laplace` files to plain `.stan`. This repo adds a real
language server plus a VS Code extension so `.laplace`/`.laplacelib`/`.stan`
files get proper block-role coloring, function-origin coloring, autocomplete,
and live diagnostics instead of generic Stan syntax highlighting.

Both source dialects the compiler accepts are supported: `.laplace` project
files (the usual Stan block structure) and `.laplacelib` library files (the
relaxed dialect -- bare function definitions, an optional `functions { }`
wrapper, an optional `library { }` block, and none of the model-shaped
blocks). The language-server-backed features below are wired to `.laplace`
only for now: laplace-lsp treats every document it is handed as a whole
`.laplace` program, so pointing it at a functions-only `.laplacelib` file
would make its `stanc` pass report a valid library as an invalid Stan
program. `.laplacelib` files get the grammar, the icon and the language
configuration; widening the server to them is separate, server-side work.

Two pieces, built independently:

- **`laplace-lsp/`** — the language server (Rust, [`tower-lsp`](https://github.com/ebkalderon/tower-lsp)).
  Reuses `laplace`'s `parser`/`resolve`/`docs`/`codegen` modules directly as a
  library dependency — no reimplemented parsing logic.
- **`vscode-laplace/`** — a thin VS Code extension (TypeScript) that spawns
  `laplace-lsp` over stdio. Works unmodified in [Positron](https://positron.posit.co/)
  once published to Open VSX, since Positron is a Code OSS fork that installs
  extensions from there — no Positron-specific code exists or is needed.

## What you get

- **Block-role coloring** — identifiers declared in `data`, `parameters`,
  `transformed parameters`, and `generated quantities` blocks are colored
  distinctly, including every usage inside `model {}`, not just the
  declaration site. (`transformed data` locals and Stan-native locals share a
  fifth "local" color, since Stan's own type system doesn't distinguish them
  further.)
- **Function-origin coloring** — a call site is colored differently depending
  on whether it's a Stan builtin (`normal_lpdf`, `gp_exp_quad_cov`, ...), a
  `pkg::func()` library import, or a function defined in the current file's
  `functions {}` block.
- **Autocomplete** — variables in scope, block/section keywords, and (typing
  `pkg::`) every exported function of that resolved library.
- **Live diagnostics** — an unresolved `pkg::func`, a missing/uninstalled
  library dependency, or a version pin that no longer matches `laplace.lock`
  is flagged inline, debounced (~350ms after the last edit, not per
  keystroke) rather than only surfacing at `laplace build` time.
- **Live Stan-level syntax/type checking** — missing semicolons, unknown
  types, incompatible operand types, and everything else real `stanc` would
  catch, also inline and debounced. Best-effort: this shells out to `stanc`
  (found via `$LAPLACE_STANC`, `PATH`, or the newest `~/.cmdstan/cmdstan-*`),
  so it silently contributes nothing if `stanc`/cmdstan isn't installed --
  the checks above never depend on it. An error inside code spliced in from
  an imported package (rather than your own file) is attributed to that
  package's `import` statement, since it has no position in your file to
  point at.
- **Hover + go-to-definition** on `pkg::func` (via the `docs`/`resolve`
  modules), as a low-marginal-cost bonus on top of the same symbol tables.
- **Baseline TextMate grammar** as a fallback layer for the moment before the
  language server attaches (or if it isn't installed at all): `library {}`,
  `@laplace` doc comments, `pkg::func()` namespacing, and Stan's own block
  keywords. `.laplace` and `.laplacelib` share one set of patterns -- the
  `source.laplacelib` grammar is a thin wrapper that includes
  `source.laplace#common`, so highlighting can't drift between the two -- but
  they keep distinct scope names (`source.laplace` / `source.laplacelib`) and
  distinct language ids so themes, per-language settings and future
  semantic-token work can target them independently. Nothing in the shared
  patterns is scoped to an enclosing block, so a `.laplacelib` file with bare
  top-level function definitions and no `data`/`parameters`/`model` blocks
  highlights in full.
- **A `.laplace`/`.laplacelib` file icon** — a purple `Λ` glyph (`vscode-laplace/icons/laplace-lambda.svg`,
  `laplace-lambda-light.svg`), shown the same way `.R`/`.py`/`.jl` get theirs:
  an icon theme's file-extension mapping, the only slot VS Code/Positron
  actually renders to the left of the filename. There's no API for an
  extension to inject a single icon into whatever theme's already active, so
  `vscode-laplace/icons/seti/` vendors the built-in Seti icon theme
  (MIT-licensed, `ThirdPartyNotices.txt` alongside it) with just the
  `.laplace`/`.laplacelib` mappings added (both point at the same icon
  definition -- one asset, two extension keys) — every other file type renders exactly as
  built-in Seti already does. Pick it via *Preferences: File Icon Theme →
  Laplace*; if you use a different icon theme day-to-day (Material Icon
  Theme, vscode-icons, ...), picking "Laplace" means other file types fall
  back to Seti's icons instead of your usual theme's.
  `vscode-laplace/icons/laplace-file-icon.png` (a cropped, background-removed
  version of the project logo, `example_logo.png`) isn't currently wired into
  anything — kept for reference/future use.

## Prerequisites

- **Rust** (stable toolchain; developed against 1.95) to build `laplace-lsp`.
- The **`laplace` core repo checked out as a sibling directory** — this repo
  depends on it as a local path dependency (`../../laplace` relative to
  `laplace-lsp/`), i.e.:

  ```
  some-parent-dir/
    laplace/         <- the core compiler (parser/resolve/docs/codegen/validate)
    laplace-tools/   <- this repo
  ```

  If your checkout lives somewhere else, either symlink it into place or edit
  the `path = "../../laplace"` dependency in `laplace-lsp/Cargo.toml`.
- **Node.js + npm** to build the VS Code extension.
- `laplace` itself installed and usable (`laplace install`/`laplace add`) in
  any project you want live diagnostics/`pkg::` completion for — the
  language server reads the same `~/.laplace/packages/` cache and
  `laplace.lock` the CLI writes; it never re-implements resolution.
- **Optional: `stanc`/cmdstan**, for live Stan-level syntax/type checking.
  Not required for anything else here. The language server looks for it in
  `$LAPLACE_STANC`, then `stanc` on `PATH`, then the newest
  `~/.cmdstan/cmdstan-*/bin/stanc` (how [cmdstanr](https://mc-stan.org/cmdstanr/)/[cmdstanpy](https://mc-stan.org/cmdstanpy/)
  typically install it, without putting it on `PATH`).


## Installing the extension for local development

Two options:

**A. Run it in an Extension Development Host** (fastest for iterating): open
`vscode-laplace/` in VS Code and press `F5`. A new VS Code window launches
with the extension active; open any `.laplace` file in it.

**B. Package and install a `.vsix`** (closer to a real install, and what you'd
hand to a colleague or sideload into Positron):

```sh
cd vscode-laplace
npx @vscode/vsce package --no-dependencies
code --install-extension laplace-lang-0.1.0.vsix
```

Either way, if `laplace-lsp` isn't on `PATH`, set its location explicitly in
VS Code settings:

```json
{
  "laplace.serverPath": "/absolute/path/to/laplace-lsp"
}
```

## How project context is resolved

The language server never asks the client for a workspace root. For any open
`.laplace` file it walks upward from the file's directory looking for the
nearest `laplace.lock`, then reads installed packages from
`~/.laplace/packages/<name>/<version>/` — the exact layout `laplace
install`/`add`/`update` already write. If no lockfile is found, diagnostics
and `pkg::` completion simply have nothing to resolve against (no crash, no
false positives) until the file sits under a real laplace project.

Full `stanc` type-checking (`laplace::validate`) is **not** wired into live
diagnostics: it shells out to an external `stanc` binary that isn't
guaranteed to be installed, which is exactly why the CLI itself only runs it
behind an opt-in `laplace build --validate` flag. Live diagnostics stick to
what `codegen` and the lockfile can check without any external dependency.

## Known limitations

- The declaration scanner is shallow by design (see the task's non-goals) —
  it extracts a name and its owning block, never a type, dimension, or
  constraint. It does not track `model {}`-local variables (Stan's grammar
  doesn't reserve a role for those beyond the generic "local" bucket, and
  scope-tracking real locals would edge toward type inference).
- Go-to-definition on `pkg::func` does a best-effort first-occurrence text
  search within the resolved package's `.stan` source (after confirming via
  the same signature scanner that the function genuinely exists there) — it
  is not a real reference resolver, and can point at the wrong occurrence if
  a package calls a function before its own definition in the same file.
- `stan-builtins.json` is a curated snapshot of Stan's function reference
  (~560 names, covering scalar math, linear algebra, ODE/algebra solvers, and
  the common distribution families with their `_lpdf`/`_lpmf`/`_cdf`/`_lcdf`/
  `_lccdf`/`_rng` suffixes), not a scrape of `stanc`'s canonical function
  table — a genuinely new or obscure builtin may show up unclassified rather
  than misclassified.

## Non-goals

Carried over unchanged from the task this was built against — flag these as
separate future work if they come up, don't try to bolt them onto this
codebase:

- Distribution-aware completion/diagnostics (e.g. validating `normal`'s
  argument types or support domain).
- Rename, find-references, call hierarchy, extract-function refactors.
- Formatting.
