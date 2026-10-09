# Laplace Language Support

Syntax highlighting, semantic coloring, autocomplete, and live diagnostics
for `.laplace` files — a source-to-source preprocessor for
[Stan](https://mc-stan.org/). Works unmodified in both VS Code and
[Positron](https://positron.posit.co/) via Open VSX.

## Features

- **Syntax highlighting** for `.laplace` files, including `library {}`
  blocks, `@laplace` doc comments, and `pkg::func()` namespaced imports.
- **Semantic block-role coloring** — identifiers declared in `data`,
  `parameters`, `transformed parameters`, and `generated quantities` are
  colored distinctly, including at every usage site (e.g. inside `model {}`),
  not just at their declaration.
- **Function-origin coloring** — Stan built-in functions, functions
  imported from a library (`pkg::func`), and functions defined in the
  current file's `functions {}` block are each colored differently.
- **Autocomplete** for library functions after typing `pkg::`.
- **Live diagnostics** for unresolved imports, missing libraries, and
  version conflicts — surfaced as you type/save, without needing to run
  `laplace build` manually.
- **Rendered `//@math` formulas** in hover tooltips. A doc comment's
  `//@math` section is LaTeX; instead of showing it as raw source, the
  extension renders it with MathJax (bundled — no network) and colours it
  for the active theme.
- **Laplace patch-1 syntax** — highlighting, snippets and diagnostics; see
  below.
- **Math symbols (optional, off by default)** — show `sigma_obs` as σ with a
  subscript, `sum(x)` as ∑(x), `<=` as ≤, and type them LaTeX-style with
  `\sigma_{obs}`. Display only; see below.

## Patch-1 syntax

The four constructs below are coloured, folded, have snippets, and are
checked as you type by the same compiler code that builds them (it needs the
`laplace` compiler with patch 1, which `laplace-lsp` is built against).

| Construct | Example |
|---|---|
| Visibility | `pub real mean(vector x) { ... }` |
| Functions as arguments | `real apply_twice(real x, func(real) -> real f)` |
| Type placeholder and accessors | `matrix[n, @wait(f).size] out;` |
| Sized return types | `vector[2] to_pair(real x) { ... }` |
| Templates | `pub @template ncp($name: ident, $N: expr) { ... }` — used as `@use pkg::ncp(theta, K);` |
| Statement macros | `pub @macro priors(each $p: ident, $dist: expr) : stmt in model { ... }` — used as `@expand pkg::priors([alpha, beta], normal(0, 1));` |

### Diagnostics

Mistakes in patch-1 syntax are reported where they are written:

| Reported | Where |
|---|---|
| `@use` / `@expand` of a name the package does not define, or did not mark `pub` | on the `@use` / `@expand` line |
| wrong argument count, an argument of the wrong kind, a macro expanded in a block its `in` list does not name | on the `@use` / `@expand` line |
| two expansions declaring the same name | at the top of the file — the compiler names both expansions in the message, but not a line |
| a malformed `@template` or `@macro`, a misplaced `pub`, a macro whose statements cannot go in its target blocks | on the definition, in the `.laplacelib` |
| a higher-order function that cannot be specialized, a bad `@wait(f)` | where the compiler points: the definition, or the call that binds it |
| an identifier containing `__`, which laplace reserves for the names it generates | on the identifier |

`stanc` checks the expanded program, so an ordinary Stan mistake is reported
wherever it sits — below a `@use`, inside a model that expands a macro, or
after a template in a library. Where `stanc` is not installed, only the
laplace-level checks above run.

### Snippets

| Prefix | File type | Inserts |
|---|---|---|
| `pubfn` | `.laplacelib` | a `pub` function |
| `hof` | `.laplacelib` | a function with a `func(...) -> ...` parameter |
| `template` | `.laplacelib` | a `@template` skeleton with an `ident` and an `expr` placeholder |
| `macro` | `.laplacelib` | a `@macro` skeleton with `each` and `: stmt in model` |
| `use` | `.laplace` | `@use pkg::name(...);` |
| `expand` | `.laplace` | `@expand pkg::name([...], ...);` |

## Math symbols

An optional display mode, for a model that reads like the maths you wrote on
paper before coding it. Turn it on with **Laplace: Toggle math symbol display**
or the `laplace.symbols.enabled` setting; it is off by default.

**It never changes the file.** Neither the compiler nor `stanc` accepts `σ` in
a name, so the file keeps `sigma_obs` and the editor draws σ<sub>obs</sub> over
it. Turning the setting off shows the file exactly as written, immediately.

| In the file | Shown as |
|---|---|
| `sigma`, `mu`, `theta`, `Sigma`, `Omega` … | σ, μ, θ, Σ, Ω … |
| `sigma_obs`, `theta_raw` | σ<sub>obs</sub>, θ<sub>raw</sub> — everything after the first `_` is the subscript, as LaTeX sets `\sigma_{obs}` |
| `sum(x)`, `prod(x)`, `sqrt(x)`, `pi()` | ∑(x), ∏(x), √(x), π() — the parentheses stay |
| `<=`, `>=`, `!=` | ≤, ≥, ≠ |

Typing works LaTeX-style: type `\` and the suggestions list the commands with
their symbols. `\sigma` inserts `sigma`; `\sigma_{obs}` (or `\sigma_y`) inserts
`sigma_obs` (`sigma_y`); `\leq`, `\geq`, `\neq` insert `<=`, `>=`, `!=`; `\sum`,
`\prod`, `\sqrt` insert the function name; `\pi` inserts `pi()`.

What is left alone: comments and strings (a `//@math` section is LaTeX in its
own right), `pkg::` names, `$placeholders`, and Stan functions that share a
Greek name — `beta(a, b)` and `gamma(a, b)` stay as they are. The line your
cursor is on always shows the real text, so you edit the actual characters.

Limits:

- VS Code has no supported way to hide text, so this uses the usual
  workaround (CSS injected through a decoration). If an editor update breaks
  it you will see the plain names, or both; turn the setting off.
- Column numbers in error messages count the real text, so they will not
  match what you see on a line full of symbols.
- A drawn symbol takes the editor's plain text colour, not the semantic colour
  of a parameter or data variable.
- Search for `sigma`, not `σ`: the file contains the name.

## Requirements

This extension is a thin client — the actual language intelligence comes
from **`laplace-lsp`**, a separate binary that must be available on your
system.

1. Build it from the [laplace_tools](https://github.com/mlatinov/laplace_tools)
   repo:
   ```bash
   cd laplace-lsp
   cargo build --release
   ```
2. Either:
   - Install it so it's on your `PATH`:
     ```bash
     cargo install --path .
     ```
   - Or point the extension directly at the built binary via the
     `laplace.serverPath` setting (see below) — useful if you don't want
     it globally on `PATH`.

If `laplace-lsp` isn't found, syntax highlighting still works (it's a
static grammar), but semantic coloring, autocomplete, and diagnostics
will not.

## Extension Settings

This extension contributes the following settings:

| Setting                  | Default        | Description                                                                 |
|---------------------------|-----------------|------------------------------------------------------------------------------|
| `laplace.serverPath`      | `laplace-lsp`   | Path to the `laplace-lsp` executable. Defaults to resolving `laplace-lsp` on `PATH`. Set an absolute path if it isn't on `PATH`. |
| `laplace.trace.server`    | `off`           | Trace communication between the editor and `laplace-lsp` (`off` \| `messages` \| `verbose`) — useful for debugging the extension itself, not your `.laplace` code. |
| `laplace.docs.renderMath` | `true`          | Render a doc comment's `//@math` section as a formula in hovers. Set to `false` to see the LaTeX source as written. |
| `laplace.symbols.enabled` | `false`         | Draw math symbols over plain names (σ for `sigma`, ∑ for `sum(`, ≤ for `<=`) and offer `\sigma`-style input. Display only — the file is never changed. See [Math symbols](#math-symbols). |

## Commands

Both are in the Command Palette (`Ctrl+Shift+P`) under **Laplace**, as a
shortcut for the corresponding setting:

| Command | Does |
|---|---|
| `Laplace: Toggle rendering of //@math doc formulas` | flips `laplace.docs.renderMath` |
| `Laplace: Toggle math symbol display (σ for sigma)` | flips `laplace.symbols.enabled` |

They write to the workspace when one is open, so the choice does not leak into
your other projects.

## Known limitations

This is an early release. Not yet supported:

- Full Stan-level type checking (e.g. catching a distribution called with
  a mismatched argument type) — current diagnostics cover laplace-level
  import/library resolution, not Stan semantic correctness.
- Rename, find-references, and call hierarchy.
- Document formatting.
- Distribution-aware completion (argument types/signatures for `normal`,
  `poisson`, etc.).
- Completion, hover and go-to-definition for patch-1 syntax: template and
  macro names are not completed after `@use pkg::` / `@expand pkg::`, and
  `@use` / `@expand` do not jump to the definition. Diagnostics are
  complete; navigation is not there yet.
- `//@math` rendering in completion documentation and signature help.
  `laplace-lsp` puts only a function's `@brief` in a completion item and
  has no signature-help provider, so the hover is currently the only place
  a formula can appear.

## Developing

```bash
npm install
npm test      # tsc --noEmit, then unit tests, then grammar snapshots
npm run package   # produces the .vsix
```

The extension is bundled with esbuild into a single `out/extension.js`, so
`vsce package --no-dependencies` is correct: nothing is resolved from
`node_modules` at runtime. Highlighting is covered by
`vscode-tmgrammar-snap` snapshots under `fixtures/`; regenerate them with
`npm run test:grammar:update` after an intentional grammar change, and read
the diff.

## Related

- [laplace](https://github.com/mlatinov/laplace) — the compiler/preprocessor itself
- [laplace_tools](https://github.com/mlatinov/laplace_tools) — this extension's source, including `laplace-lsp`

## License

MIT
