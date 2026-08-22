# Task: laplace-lsp — Language Server for .laplace files (VS Code + Positron)

## Context
laplace is a source-to-source preprocessor compiling `.laplace` files to `.stan`.
The implementation already exists at /home/metodi/Data_Science_Projects/laplace
(~3885 lines, Rust) with working modules: parser, codegen, resolve, docs, validate.
The full pipeline (`laplace add`, `laplace install`, `laplace build`) works end to end.
Read CLAUDE.md and laplace-project-plan.md at the repo root before starting.

The current pain point driving this task: editing `.laplace`/`.stan` files in
VS Code / Positron uses generic Stan syntax highlighting, which colors `data`,
`parameters`, `transformed parameters`, and `generated quantities` declarations
identically, and gives no distinction between Stan built-in functions, imported
laplace library functions (`pkg::func`), and user-defined functions. Autocomplete
and live validation are effectively absent.

## Goal
Build a real language server (`laplace-lsp`) and a matching VS Code extension
that also works unmodified in Positron (Positron is a Code OSS fork that installs
extensions via Open VSX — publish there in addition to, or instead of, the VS
Code Marketplace; no Positron-specific code should be needed).

## Explicit scope (what must ship)

### 1. Semantic token classification — block-role coloring
Implement a shallow declaration scanner (NOT a type checker):
- For each file, walk top-level blocks (`data`, `parameters`, `transformed data`,
  `transformed parameters`, `model`, `generated quantities`, `functions`,
  laplace's `library {}`).
- Within `data`/`parameters`/`transformed data`/`transformed parameters`/
  `generated quantities`, extract declared identifier names (ignore/skip type
  constraints and dimension expressions — only the declared name and its
  owning block matter).
- Build one symbol table per file: `name -> role` where role is one of
  `data | parameter | transformedParameter | generatedQuantity | local`.
- Walk the token stream again and emit LSP semantic tokens tagging every
  identifier occurrence (not just the declaration site) with its role, so
  usages inside `model {}` are colored according to where they were declared.
- Register a semantic token legend with these custom types so the client can
  map each to a distinct color.

Do NOT attempt expression type inference, dimension checking, or constraint
validation as part of this — role classification only.

### 2. Semantic token classification — function origin
Classify every function call site into one of three categories and emit as a
distinct semantic token type:
- `pkg::func()` syntax → library function (already unambiguous from the
  existing parser)
- name declared in the current file's `functions {}` block → user-defined
- name matching a bundled static list of Stan's built-in function names →
  builtin (ship this as a data file, e.g. `stan-builtins.json`, sourced once
  from Stan's function reference — not derived by parsing Stan's math library)

### 3. Autocomplete
Powered by the same symbol tables as #1/#2, plus the existing `resolve` module:
- Complete variable names currently in scope
- Complete `pkg::` → list exported functions of that resolved library
  (via `resolve`)
- Complete block/section keywords (`data`, `parameters`, `library`, etc.)

### 4. Live diagnostics
Wire the existing `validate` module to run on file change/save (debounced,
not per-keystroke) and publish results as LSP diagnostics instead of only
surfacing at `laplace build` time:
- unresolved `pkg::func`
- missing/unresolved library dependency
- version conflicts against `laplace.lock`
- any other error class `validate` already produces

Full Stan-level type checking (e.g. "normal expects a scalar, got vector[5]")
is explicitly OUT OF SCOPE for this task — flag it as a separate future task
if it comes up, do not attempt to reimplement Stan's type checker here.

### 5. Baseline syntax highlighting
A TextMate grammar for `.laplace` files covering `library {}`, `@laplace` doc
comments, `pkg::func()` namespacing, and passthrough Stan block keywords — this
is the fallback layer for editors/moments where the LSP hasn't attached yet.

### 6. Editor integration
- `laplace-lsp`: new Rust binary (or crate) using `tower-lsp`, reusing
  `parser`/`resolve`/`docs`/`validate` directly — no duplicated logic in
  TypeScript.
- VS Code extension: thin client that spawns `laplace-lsp` over stdio and
  registers it for `.laplace` files (and optionally generated `.stan` files
  if useful).
- Publish to both the VS Code Marketplace and Open VSX (`ovsx publish`) so
  Positron picks it up with no separate integration work.

## Nice-to-have (not essential — include only if low additional cost)
- User-configurable coloring/theme: expose the semantic token types cleanly
  enough that users can override colors via standard
  `editor.semanticTokenColorCustomizations` in their own settings, and ship
  one sensible default color mapping bundled with the extension (not forced).

## Bonus if infrastructure makes it near-free
Since building the LSP server and symbol tables is required for #1-#4 anyway,
also wire up (low marginal cost, reuses the same tables/modules):
- Hover on `pkg::func` showing `@laplace` doc comments (via `docs` module)
- Go-to-definition on `pkg::func` (via `resolve` module, which already maps
  this to a source location)

## Explicit non-goals for this task
- Full Stan type system / type checking (would require reimplementing large
  parts of `stanc3` — separate future task, likely via shelling out to real
  `stanc` with a source map, not hand-built)
- Distribution-aware completion/diagnostics (`normal` argument types, support
  domains)
- Rename, find-references, call hierarchy, extract-function refactors
- Formatting

## Acceptance criteria
- Opening a `.laplace` file in VS Code shows data/parameters/transformed
  parameters/generated quantities identifiers in visually distinct colors,
  including at usage sites inside `model {}`, not just at declarations
- Builtin, library (`pkg::func`), and user-defined functions are visually
  distinct
- Typing `pkg::` triggers completion listing that library's exported functions
- Editing an import that doesn't resolve produces an inline diagnostic without
  running `laplace build` manually
- The same `.vsix` (or its Open VSX publish) works unmodified when installed
  in Positron
