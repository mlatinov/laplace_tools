# Patch 1: follow-up now that the compiler accepts the new syntax

The editor tooling for Laplace patch-1 syntax (`pub`, `func(...) -> ...`,
`@wait(...)`, sized return types, `@template`/`@use`, `@macro`/`@expand`)
shipped **ahead of the compiler** (2026-09-29, `vscode-laplace` 0.3.0), with
workarounds for a compiler that rejected the syntax. The compiler caught up in
`laplace` commit `65103d1` (2026-10-08). This file was the checklist for
undoing the workarounds; §0–§3 and §6 record how that went (2026-10-09,
`vscode-laplace` 0.4.0). §5, §7 and §8 are still live.

---

## 0. Tripwires — resolved

| Test | Outcome |
|---|---|
| `a_macro_headers_in_clause_looks_like_a_forbidden_block` | still passed: the LSP called `find_top_level_blocks` itself (§1a). Inverted to `a_macro_headers_in_clause_is_not_a_forbidden_block` |
| `a_use_of_a_template_looks_like_an_unexported_function` | failed, as intended (§1b). Replaced by tests that `@use`/`@expand` resolve, and that an unknown or private one is reported on its line |
| `the_rest_of_patch_1_produces_no_diagnostics_at_all` | still true; now `a_library_using_all_of_patch_1_is_clean`, with a `@macro` added |
| `math_is_lifted_out_of_real_rendered_output` (`hover.rs`) | **kept** — it guards `laplace::docs`'s render format, nothing to do with patch 1 |

`laplace-lsp` also stopped compiling, as §7 predicted: `CodegenError` lost
`PrivateFunctionCollision` and gained eight variants, and `FunctionSig` gained
seven fields.

## 1. The two compiler fixes

**1b (`@use` / `@expand`) — fixed in the compiler.** codegen resolves templates
and macros as their own kind of export.

**1a (`@macro` header) — fixed in the compiler, but not where this file said.**
`find_top_level_blocks` is unchanged; `laplacelib::parse` instead drops blocks
that start inside a template or macro definition. Anything calling
`find_top_level_blocks` directly still sees `in model {` as a `model` block,
so `laplace-lsp` does the same filtering in two places:
`diagnostics::forbidden_block_diagnostics`, and `stanc::wrap_library_source`,
which now builds on `laplacelib::parse`.

## 2. Stan-level checking — restored

Re-verified with a real `stanc` (cmdstan 2.39):

| shape | before | now |
|---|---|---|
| model, `@use` above or below a missing `;`; `@expand` above one | only `unknown-export` | `stanc` reports the `;` |
| model with no patch-1 syntax (control) | `stanc` reports it | unchanged |
| library, error below a template, macro, HOF and sized return | nothing (macro header read as a forbidden block, so no `stanc` run) | `stanc` reports it on the right line |
| clean library using `pub` | false "Ill-formed block" at `pub` (hidden by the extension's filter) | clean |

Models were fixed by §1b alone. Libraries needed the LSP change above: the raw
text, `pub` and all, used to go straight to `stanc`; now it is what
`laplacelib::parse` emits, with HOFs and sized returns left for codegen.
`stanc.rs` has real-`stanc` tests for both cases, which skip when no `stanc`
is installed.

## 3. The diagnostics filter — deleted

All of it, as listed: `diagnosticFilter.ts` and its test, the
`handleDiagnostics` middleware, the status-bar indicator, the
`laplace.diagnostics.unsupportedSyntax` setting and its toggle command, their
`manifest.test.ts` assertions, and the README sections. One spot the list
missed: `esbuild.js` names each test file.

What replaced it: diagnostics the filter used to hide are gone at the source,
and the LSP reports the compiler's patch-1 errors itself, under new codes
`private-item`, `reserved-identifier` (`__`, previously §5), `library-item`,
`expansion` and `higher-order`. Expansion errors are placed with the
compiler's own `--> <source>:line:col`. One gap: `Collision` / `OutOfOrder`
(two expansions declaring one name) carry no location, so they land at the top
of the file.

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
- ~~Real diagnostics~~ and ~~`__` in identifiers~~ — done in 0.4.0, see §3.

---

## 6. Grammar details — checked against the compiler

- **Wrapped macro headers: the compiler accepts them** (targets run up to the
  body's `{`). `#macro-target-blocks` now ends at `(?=[{;}])` instead of
  end-of-line, with a fixture for a header wrapped inside its block list. A
  break directly after `in` still is not highlighted: the `begin` lookahead
  cannot see the next line.
- Placeholder kinds: the compiler has `ident` and `expr`; macro kinds: `stmt`.
  Matches the grammar, which also colours the reserved `type`, `decl`, `expr`
  — and the LSP now reports them as errors, which is the right signal.
- Sized return types: the compiler's form matches the grammar's.
- `func` is not reserved (`parser::functional::FUNC_KEYWORD` is only matched
  in a parameter type), so `\bfunc\s*\(` stays as is.

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
