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
- **Highlighting for Laplace patch-1 syntax** — see below.

## Patch-1 syntax

**Highlighting support only — compiler support is coming in the next
release.** The four constructs below are coloured, folded, and have
snippets, so a library can be written in the new syntax with the editor
keeping up. The Laplace compiler does not accept any of them yet, and this
extension adds no semantic understanding of them: no completion, no
go-to-definition, no type checking.

| Construct | Example |
|---|---|
| Visibility | `pub real mean(vector x) { ... }` |
| Functions as arguments | `real apply_twice(real x, func(real) -> real f)` |
| Type placeholder and accessors | `matrix[n, @wait(f).size] out;` |
| Sized return types | `vector[2] to_pair(real x) { ... }` |
| Templates | `pub @template ncp($name: ident, $N: expr) { ... }` — used as `@use pkg::ncp(theta, K);` |
| Statement macros | `pub @macro priors(each $p: ident, $dist: expr) : stmt in model { ... }` — used as `@expand pkg::priors([alpha, beta], normal(0, 1));` |

Because the compiler is still the old one, `laplace-lsp` reports two things
about patch-1 files that are artefacts of its age rather than real problems:
a `@macro` header's `in model { ... }` clause looks like a forbidden `model`
block in a `.laplacelib`, and the template or macro named by `@use` /
`@expand` looks like a function the package does not export. Set
`laplace.diagnostics.unsupportedSyntax` to `false` to hide those (and the
`stanc` pass, which rejects the whole generated model) while rewriting a
library. Unresolved imports, version conflicts and uninstalled packages are
always reported.

### Snippets

| Prefix | File type | Inserts |
|---|---|---|
| `pubfn` | `.laplacelib` | a `pub` function |
| `hof` | `.laplacelib` | a function with a `func(...) -> ...` parameter |
| `template` | `.laplacelib` | a `@template` skeleton with an `ident` and an `expr` placeholder |
| `macro` | `.laplacelib` | a `@macro` skeleton with `each` and `: stmt in model` |
| `use` | `.laplace` | `@use pkg::name(...);` |
| `expand` | `.laplace` | `@expand pkg::name([...], ...);` |

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
| `laplace.diagnostics.unsupportedSyntax` | `true` | Show the diagnostics that patch-1 syntax provokes from a compiler that does not accept it yet. Set to `false` while rewriting a library in the new syntax. Import and lockfile diagnostics are always shown. |

## Commands

Both of these are in the Command Palette (`Ctrl+Shift+P`) under **Laplace**, as
a shortcut for the corresponding setting:

| Command | Does |
|---|---|
| `Laplace: Toggle diagnostics from unsupported (patch-1) syntax` | flips `laplace.diagnostics.unsupportedSyntax` |
| `Laplace: Toggle rendering of //@math doc formulas` | flips `laplace.docs.renderMath` |

They write to the workspace when one is open, so hiding diagnostics while
rewriting one library does not leak into your other projects.

While patch-1 diagnostics are hidden, a warning appears in the status bar --
suppressing them also suppresses the whole `stanc` pass, so it is worth being
able to see at a glance that Stan-level checking is off. Click it to turn them
back on.

## Known limitations

This is an early release. Not yet supported:

- Full Stan-level type checking (e.g. catching a distribution called with
  a mismatched argument type) — current diagnostics cover laplace-level
  import/library resolution, not Stan semantic correctness.
- Rename, find-references, and call hierarchy.
- Document formatting.
- Distribution-aware completion (argument types/signatures for `normal`,
  `poisson`, etc.).
- Anything semantic about patch-1 syntax — it is highlighted, but not
  understood: no completion for template or macro names, no
  go-to-definition on `@use` / `@expand`, and no checking that a
  placeholder is bound or that a macro's target blocks exist.
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
