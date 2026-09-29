# Patch 1: what to change once the compiler accepts the new syntax

The editor tooling for Laplace patch-1 syntax (`pub`, `func(...) -> ...`,
`@wait(...)`, sized return types, `@template`/`@use`, `@macro`/`@expand`)
shipped **ahead of the compiler**, deliberately. A fair amount of what it
contains exists only to paper over a compiler that rejects the syntax, and
becomes dead weight the moment that changes.

This file is the list of what to undo, what to keep, and what becomes possible.
Written 2026-09-29, against `vscode-laplace` 0.3.0 and the `laplace` crate at
`../laplace` (a separate repo, reached by the `laplace-lsp` path dependency
`../../laplace`).

---

## 0. Tripwires: tests that are *supposed* to fail

These assert that the compiler mishandles patch-1 syntax. When it stops doing
so, they fail — which is the point. Do not "fix" them by loosening the
assertion; delete or invert them, then work through this file.

| Test | In | Asserts |
|---|---|---|
| `a_macro_headers_in_clause_looks_like_a_forbidden_block` | `laplace-lsp/src/diagnostics.rs` | a `@macro` header's `in model {` is reported as a forbidden block |
| `a_use_of_a_template_looks_like_an_unexported_function` | `laplace-lsp/src/diagnostics.rs` | `@use pkg::name` is reported as an unexported function |
| `the_rest_of_patch_1_produces_no_diagnostics_at_all` | `laplace-lsp/src/diagnostics.rs` | `pub`, `func(...)`, `@wait`, placeholders, sized returns produce nothing |
| `math_is_lifted_out_of_real_rendered_output` | `laplace-lsp/src/hover.rs` | couples to `laplace::docs`'s terminal format; fails if that is reformatted |

---

## 1. Two compiler fixes the editor cannot work around

### 1a. `find_top_level_blocks` and the `@macro` header

`laplace::parser::blocks::find_top_level_blocks` walks braces and matches a
block keyword followed by `{` at depth 0. A macro header ends
`... : stmt in model {`, which puts `model {` at depth 0, so it is
indistinguishable from a real `model` block. In a `.laplacelib` that produces:

> a `.laplacelib` file cannot contain a `model` block

**Any** `in <blocks> {` triggers it, not just `model` — the last block name
before the brace is the one that matches.

Fix in the compiler: skip the `in` clause of a `@macro` header. Then
`forbidden_block_diagnostics` in `laplace-lsp/src/diagnostics.rs` is correct
again with no client-side help, and the `forbidden-block` rule in the
extension's filter can go.

### 1b. `find_qualified_calls` and `@use` / `@expand`

`laplace::codegen::rename::find_qualified_calls` sees
`@use pkg::name(theta, K);` as a call to a function `name` in `pkg`. Since a
template is not a function export, codegen fails with `FunctionNotExported`.

This is the bad one, because of what it cascades into — see §2.

Fix in the compiler: resolve templates and macros as their own kind of export,
or exclude `@use` / `@expand` operands from the function-export check.

---

## 2. Models currently get **no** Stan-level checking (and libraries partly do)

Two different mechanisms, verified against the live server. Worth keeping
straight, because they behave differently and only one has a workaround.

**Fact that explains both: `stanc` reports only one error per run.** It stops at
the first parse error. Verified on a file with two real mistakes and no patch-1
syntax: only the first was reported.

### Libraries (`.laplacelib`)

codegen succeeds (nothing references `pkg::`), so `stanc` runs and stops at the
first patch-1 construct.

- an error **above** the first patch-1 line is reported normally
- an error **below** it is not — `stanc` never got there

Workaround that works today: keep templates and macros at the bottom of the
file, or in files of their own.

### Models (`.laplace`)

codegen **fails** on the `@use` / `@expand` (§1b), and
`stanc::compute_stanc_diagnostics` returns early when codegen fails. So `stanc`
is never invoked and the whole file loses Stan checking — regardless of where
the directive sits. No workaround; a model cannot move its `@use` out of itself.

Verified:

| file | shape | result |
|---|---|---|
| `m1` | missing `;` line 6, `@use` line 10 | only `unknown-export`; the `;` is **not** reported |
| `m2` | `@use` line 5, missing `;` line 8 | same |
| `m3` | `@expand` in `model {}`, missing `;` after | same |
| `m4` | control, no `@use` | `stanc`: "Ill-formed declaration. Expected ';'" ✅ |

**Fixing §1b fixes this outright**: codegen succeeds, `stanc` runs, the model is
checked. Re-verify with the `m1`–`m4` shapes above.

A tempting LSP-side workaround was considered and rejected: blank out the
`@use` / `@expand` statements (space-padded to preserve offsets) before the
`stanc` pass only. codegen would then succeed, but a `@use` injects
declarations the model goes on to use, so `stanc` would report those as
undefined variables — trading no checking for a fresh class of false positive.
Not worth building for something the compiler removes.

---

## 3. Delete: the diagnostics filter and everything around it

Once §1a and §1b are fixed, patch-1 syntax stops provoking diagnostics and this
all becomes dead code.

- **`vscode-laplace/src/diagnosticFilter.ts`** — delete the module. Note it also
  contains `firstUnsupportedLine` / `lineHasUnsupportedSyntax`, including a
  regex for sized return types; nothing else uses them.
- **`vscode-laplace/src/test/diagnosticFilter.test.ts`** — delete.
- **`vscode-laplace/src/extension.ts`** — remove `handleDiagnostics` middleware,
  `documentContext`, `createSuppressionIndicator`, `refreshIndicator`, the
  `laplace.toggleUnsupportedSyntaxDiagnostics` handler, `UNSUPPORTED_SYNTAX`,
  and the `client.restart()` branch in `onDidChangeConfiguration`. Keep
  `toggle()` if the math command stays.
- **`vscode-laplace/package.json`** — remove the setting
  `laplace.diagnostics.unsupportedSyntax` and the command
  `laplace.toggleUnsupportedSyntaxDiagnostics`.
- **`vscode-laplace/src/test/manifest.test.ts`** — update the expected command
  and settings lists (it asserts both exist).
- **`vscode-laplace/README.md`** — remove the "Diagnostics while the compiler
  catches up" section and the status-bar paragraph; change "highlighting
  support, compiler support coming in the next release" to what is then true.

Removing a published setting is breaking for anyone who set it. A user with
`"laplace.diagnostics.unsupportedSyntax": false` in their settings will get an
"unknown configuration" warning, nothing worse.

## 4. Keep

- **`laplace-lsp` diagnostic codes** (`diagnostics::code`). Good practice
  regardless — a client should not have to match on message prose. The
  `unknown-export` / `forbidden-block` codes stay meaningful for the genuine
  errors they also cover.
- **The whole TextMate grammar**, snippets, `wordPattern`, fixtures and
  snapshots. Highlighting does not depend on the compiler.
- **`//@math` rendering**, the ```` ```math ```` fence in `laplace-lsp/src/hover.rs`,
  and `laplace.docs.renderMath`. Unrelated to patch 1.
- **`scripts/check-bundle.js`** and the esbuild `PACKAGE_VERSION` define. See §7.

---

## 5. What becomes possible (the interesting part)

Patch-1 support is highlighting-only today: no completion, no
go-to-definition, no checking. Once the compiler has an AST for it:

- **Completion** for template and macro names after `@use pkg::` / `@expand pkg::`.
  `completion.rs` already does this for functions via each package's
  `docs.json`; templates and macros would need to appear there too.
- **Go-to-definition** on `@use` / `@expand` → the `@template` / `@macro`
  definition. `workspace::find_function_definition` is the existing shape to
  follow.
- **Hover** on a `@use` showing the template's expansion, and on a placeholder
  showing its declared kind.
- **Semantic tokens** distinguishing a bound placeholder from an unbound one.
- **Real diagnostics** that only the compiler can give: an unbound placeholder,
  an argument whose kind does not match (`ident` vs `expr` vs `type`), a macro
  expanded in a block absent from its `in` list, `each` applied to a non-list,
  a `pub`-less template referenced from another package.
- **`__` in user identifiers.** The compiler will reject it; the LSP could
  surface it as a diagnostic. Patch 1 deliberately added no editor-side check.

---

## 6. Grammar details to re-check against the final syntax

The grammar was written from the spec, not from a working compiler. Confirm:

- `#macro-target-blocks` ends at `(?=\{)|$`, so a macro header **wrapped across
  lines** would stop matching at the newline. Fine today; revisit if the
  compiler allows it.
- Placeholder kinds: `ident` and `expr` are used, `type` is reserved. Macro
  kinds: `stmt` is used, `decl` and `expr` are reserved. If the final names
  differ, update `#placeholder-declaration` and `#macro-kind`.
- Sized return types are described in the spec as "stripped by the compiler".
  Confirm the final form still matches
  `^\s*(?:pub\s+)?<type>\s*\[...\]\s+ident\s*\(`.
- `\bfunc\s*\(` would also match a call to a user function named `func`.
  Harmless, but if `func` becomes reserved this can be tightened.

---

## 7. Environment gotchas worth not rediscovering

- **`laplace-lsp` did not compile at `30b24e3`.** The `laplace` crate had moved
  ahead: `docs::lookup` returns `Vec<FunctionSig>` (use `render_overloads`), and
  `CodegenError` gained variants. Expect this drift again; `cargo test` in
  `laplace-lsp` is the check.
- **The path dependency `../../laplace` breaks in a git worktree**, which sits
  three levels deep. A symlink at `.claude/worktrees/laplace` pointing at the
  compiler repo makes `cargo` resolve; delete it when done.
- **Hover only fires on a `pkg::func` call site**, and needs a `laplace.lock`
  findable from the file plus the package installed under
  `~/.laplace/packages/<name>/<version>/docs.json`. Hovering a definition, or
  anything in `fixtures/` (no lockfile), returns nothing. Not a bug.
- **`stanc` is found via `~/.cmdstan/cmdstan-*/bin/stanc`** when it is not on
  `PATH`, so `which stanc` finding nothing does *not* mean the stanc pass is
  inactive.
- **MathJax needs `PACKAGE_VERSION` defined at bundle time.** Without it, its
  `eval('require')` of its own `package.json` resolves relative to `out/` and
  activation throws `MODULE_NOT_FOUND` at the first hover — while unit tests
  pass, because `out/test/` sits a directory deeper. `scripts/check-bundle.js`
  guards this.

---

## 8. Unrelated, and not blocked on the compiler

- **`ts::arima_lpdf` has invalid LaTeX** in its `@math`: an English NOTE
  sentence including `lp__`, which in math mode is two consecutive subscripts
  ("Missing open brace for subscript"). It is the only one of 474 documented
  functions in the local package cache that fails to render. Fix in the `ts`
  library — move the prose to `@brief`, or write `lp\_\_`.
- **`laplace doc --html`** (in the compiler, `docs::wrap_html`) loads KaTeX from
  `cdn.jsdelivr.net`, so it needs network. Could be bundled locally the way the
  extension bundles MathJax.
- **`//@math` never appears in completion documentation or signature help**,
  because `laplace-lsp` puts only `@brief` in a completion item and has no
  signature-help provider. The extension's middleware would handle a fence if
  one appeared; the missing piece is server-side.
