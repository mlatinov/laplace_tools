# Phase 0 report (editor support for `.lapkernel` / `.lapsampler`)

Survey for the plan in `laplace_language_support.md`. Branch `editor/phase-0`.
No feature work: the only files added are this report and
`docs/editor/open-questions.md`. No existing file changed, and nothing in the
`laplace` repo changed. Probes ran from a throwaway crate in the session
scratchpad, outside both repos.

Surveyed against `laplace` `main` at `b2c481f` (equal to `origin/main`,
version 0.3.0) and `laplace_tools` `main` at `5f01d2f`.

**Short version.**

- **Startup bug: already fixed.** The server waits on stdin, answers
  `initialize` / `shutdown` correctly and writes nothing stray. Phase 2.1
  shrinks to adding the integration test.
- **Columns: Unicode scalar values (chars), 1-based.** Not bytes and not UTF-16.
  Phase 2 must convert chars → UTF-16 (only astral characters differ).
- **Dependency: `laplace-sample` cannot be used as a library from
  `laplace-lsp`, for two reasons.**
  - **(a) It needs libclang at build time.** `bridgestan`'s build script
    always runs `bindgen`.
  - **(b) It does not compile at all outside the `laplace` workspace.** The
    vendored nuts-rs `[patch]` only applies inside that workspace. I
    reproduced this:
    `error[E0599]: no method named refresh_position`.

  Recommended fix: a new crate, `laplace-sampler-syntax`, in the `laplace`
  workspace (details below).
- **Regression baseline.** The `.laplace` side has 42 extension unit tests and
  5 grammar snapshots, all passing. But **`laplace-lsp`'s tests do not
  compile** against `laplace` 0.3.0, and the crate is not rustfmt-clean.
  Neither problem is new.

## Done

- 0.1 Current state of `laplace_tools`: extension, LSP, startup probe (no commit, survey only)
- 0.2 Syntax inventory from the pest grammars, the registry and the column-unit probe (no commit)
- 0.3 Dependency question: manifest reading plus a real build probe (no commit)
- Report and open questions: one commit on `editor/phase-0`

## Checks

| Check | Result | Evidence |
| --- | --- | --- |
| Server waits with stdin open and no input | PASS | Held open 6 s, killed at 2 s → exit 124, 0 bytes on stdout |
| Server exits cleanly on immediate EOF | PASS | `</dev/null` → exit 0, 0 bytes |
| Leading whitespace before the first header | PASS | `\n` then idle → 0 bytes (the `SkipLeadingWhitespace` wrapper in `main.rs`) |
| Real `initialize` → `initialized` → `shutdown` → `exit` | PASS | Correct `InitializeResult` (id 1), one `window/logMessage`, `null` result (id 2), exit 0, stderr empty |
| Extension `npm test` (tsc, unit, grammar snapshots, bundle) | PASS | 42/42 unit; 5/5 snapshot fixtures |
| `laplace-lsp` `cargo build` / `cargo clippy -D warnings` (bin) | PASS | |
| `laplace-lsp` `cargo test` | **FAIL (pre-existing)** | 5× `E0063: missing field kind in initializer of LockedPackage` (`diagnostics.rs:479,511,689`, `stanc.rs:475,637`) |
| `laplace-lsp` `cargo fmt --check` | **FAIL (pre-existing)** | Diffs in 10 of 11 source files; the crate was never formatted |
| `laplace-sample` as an outside dependency | **FAIL** | See 0.3 |
| Editor API column unit | chars | See 0.2.3 |

## 0.1 Current state of `laplace_tools`

### The extension (`vscode-laplace`, 0.4.0)

- **`package.json`**
  - Languages `laplace` and `laplacelib` share one `language-configuration.json`.
  - Grammars `source.laplace` and `source.laplacelib`, with one snippet file each.
  - Semantic token types plus scope fallbacks for both languages.
  - Icon theme; settings `laplace.serverPath` (default `laplace-lsp` on PATH),
    `laplace.trace.server`, `laplace.docs.renderMath` and `laplace.symbols.enabled`.
  - Activation is on `onLanguage:laplace` and `onLanguage:laplacelib`.
- **Language configuration**
  - `//` line comments only, with no block comment declared.
  - Brackets `{} [] ()`, auto-closing including `"`.
  - A custom `wordPattern` (`@directives`, `${placeholder}`, `ns::name`) and
    brace-based indentation rules.
  - **No `onEnterRules`, so `//@` does not continue on Enter today.** Plan 1.1
    says to match `.laplace`, so the new languages should not continue it either.
- **LSP launch (`src/extension.ts`)**
  - `vscode-languageclient` starts `laplace.serverPath` over stdio.
  - `documentSelector` is `{scheme: file, language: laplace|laplacelib}`.
    Phase 2 adds `lapkernel` and `lapsampler` here.
- **Grammars**
  - `laplace.tmLanguage.json` has one top-level `#common` and a repository.
  - `laplacelib.tmLanguage.json` reuses the `.laplace` patterns.
  - Repository keys that could be included: `comments`, `type-keywords`,
    `control-keywords`, `constants`, `operators`, `function-call`,
    `qualified-call`, `block-keywords`, among others. All scopes end in `.laplace`.
- **Scripts**
  - `npm test` runs `tsc --noEmit`, unit tests (`node --test`), grammar
    snapshots (`vscode-tmgrammar-snap` over `fixtures/**`) and a bundle check.
  - `npm run package` builds a VSIX with `@vscode/vsce`.
- **CI: none.** `laplace_tools` has no `.github/`. Phase 4 adds it.

### The LSP (`laplace-lsp`, 0.1.0)

- **Setup**
  - `tower-lsp 0.20` on tokio, with full-document sync (`TextDocumentSyncKind::FULL`).
  - Debounced diagnostics (`schedule_diagnostics`), semantic tokens, completion
    (trigger `:`), hover and go-to-definition, all for `.laplace` and `.laplacelib`.
  - `did_close` clears diagnostics.
- **Dependency:** `laplace = { path = "../../laplace" }`. That is a path into the
  compiler's working tree, not a pinned revision.
- **Dialect routing:** `Dialect::for_path` treats every extension except
  `.laplacelib` as `.laplace`.
  - **A `.lapkernel` sent to the server today would get `.laplace`
    diagnostics.** It can't happen yet, because the client never sends one.
  - Phase 2 must add a dialect for each new extension, or route them before
    the dialect is decided.
- **Tests:** 63 unit tests across `analysis`, `diagnostics`, `stanc`, `hover`,
  `completion`, `workspace`, `dialect` and `main`. There is no end-to-end
  JSON-RPC test.
- **Startup bug status: fixed** by `SkipLeadingWhitespace` in `main.rs`, which
  has 3 unit tests.
  - The remaining behaviour belongs to tower-lsp: non-whitespace garbage before
    the first `Content-Length`, such as `hello\n`, still gets one
    `-32700 Parse error` and the server exits. Real clients never send that.
  - The release binary in `target/release` dates from 2026-10-09. The probes
    above used a fresh debug build of `main`.

## 0.2 Syntax inventory (the grammar spec for Phase 1.2)

Sources:

- `laplace-sample/src/lapkernel/grammar.pest`
- `laplace-sample/src/lapsampler/grammar.pest`
- `lapkernel/builtins.rs` (`Registry::standard`)
- `lapkernel/check.rs` (`@adapt`)
- `lapkernel/parse.rs` (`doc_comments`)
- `lapsampler/mod.rs` (`builtin_settings`, `check_kexpr`)

### 0.2.1 `.lapkernel`

| Token class | Tokens | Notes |
| --- | --- | --- |
| File head | `kernel <ident> {` | One kernel per file |
| Block keywords | `tuning`, `state`, `init`, `functions`, `step` | `tuning` and `state` hold declarations only; `init` and `step` hold statements; `functions` holds Stan-style fundefs |
| Types | `int`, `real`, `vector`, `matrix`, `tuple`; `void` (function signatures only) | **Narrower than `.laplace`**: no `array`, `simplex`, `row_vector`, `complex*`, and so on |
| Bounds | `lower`, `upper` inside `<…>` | `real<lower=0>`, `vector<lower=0>[dim]` |
| Control | `if`, `else`, `for`, `in`, `while`, `return` | **No** `break`, `continue` or `target` (`.laplace`'s `control-keywords` has them) |
| Annotation | `@adapt(` kind `[, name = expr]* )` after a tuning declaration, before `;` | Kinds: `step_size` (requires `target_accept = <0..1>`, no other args) and `metric` (no args, needs `vector[dim]`). Unknown kinds are an error |
| Context values | `q`, `lp`, `grad`, `dim`, `metric` | `metric` is both a context value and an `@adapt` kind |
| Context functions | `log_density`, `log_density_grad`, `leapfrog` | |
| Terminal statements | `propose`, `move` | Called as functions (`propose(q_new, c);`, `move(q);`); the checker requires one on every path |
| Library functions | `std_normal_rng`, `normal_rng`, `uniform_rng`, `normal_lpdf`, `std_normal_lpdf`, `dot_product`, `sum`, `rep_vector`, `exp`, `log`, `sqrt`, `square` | The full v1 registry (`Registry::standard`) |
| Operators | `\|\| && == != <= >= < > + - .* ./ * / % ^`, prefix `- !`, assignment `= += -= *= /=` | |
| Special syntax | `\|` conditional bar in call args (`normal_lpdf(x \| mu, s)`); tuple index `.N`; tuple literal `(a, b)`; destructuring `(real a, vector[dim] b) = f(...);` | |
| Literals | Integers; reals with `.` and/or exponent (`1e-3`, `.5`) | **No strings, no booleans, no `pi`/`e` constants** (`e` and `pi` are plain identifiers) |
| Comments | `//…`, `/* … */` | Both are allowed by the grammar |
| Doc tags | `//@laplace`, `//@brief <text>`, `//@param <name> <text>` | Read only **before** the `kernel` keyword; other `//@x` tags are ignored |

### 0.2.2 `.lapsampler`

| Token class | Tokens | Notes |
| --- | --- | --- |
| File head | `sampler <ident> {` | |
| Sections | `library`, `kernel`, `warmup`, `draws` | A duplicate section is an error; `kernel` is required and must not be empty |
| Library | `import <ident>[@<version>][;]` | `@version` parses, but the checker rejects it (versions come from `laplace.lock`) |
| Settings | `<ident> = <number>;` in `warmup` / `draws` | `warmup`: `iterations`; `draws`: `iterations`, `thin` |
| Kernel line | `block(<p>, …) [as <label>] : <kexpr> [;]` or `rest : <kexpr> [;]` | `rest` must be last; labels must be unique |
| Line keywords | `block`, `rest`, `as` | |
| Combinators | `seq(k, …)`, `mix(w : k, …)`, `repeat(n, k)`, `adapt(k[, target_accept = x])`, `on(block(…), k)` | `on` parses but is rejected inside a kernel line; `adapt` takes only `target_accept`; `nuts()` is not allowed under `mix` |
| Built-in kernels | `nuts`, `rwm`, `mala`, `hmc` | Settings (checker list): `nuts`: `step_size`, `target_accept`, `max_depth`; `rwm`/`mala`: `step_size`; `hmc`: `step_size`, `steps`, `jitter` |
| Namespaced kernel | `ns::name[(args)]` | `ns` must be imported |
| Local path kernel | `./…`, `../…` or `/…` up to `(` `)` `,` `;` or whitespace, then optional `(args)` | **Unquoted**: a path is not a string token |
| Arguments | `name = number \| true \| false` | |
| Numbers | Optional leading `-`, decimals, exponent | |
| Comments | `//…`, `/* … */` | No doc tags |

Two grammar traps for 1.2:

- In `.lapsampler`, `ident` does not exclude keywords, and `kernel` is also a
  section name.
- `adapt` is a combinator in `.lapsampler`, while `@adapt` is an annotation in
  `.lapkernel`.

### 0.2.3 Built-in context registry: reachable?

- **Technically yes; supported, no.** `laplace_sample::lapkernel::builtins::Registry::standard()`
  is `pub`. Each `Entry` carries `name`, `doc`, `needs_chain` and `kind`, where
  `kind` is `Value` or `Function { terminal }`.
- But `lsp-integration.md` declares only `editor::{check_lapkernel,
  check_lapsampler, Diagnostic, Severity, Project}` stable.
- **There are no readable signatures.** The type rule is a `fn` pointer. Some
  `doc` strings start with a signature (`"exp(x): …"`), others don't
  (`q` → `"current position (unconstrained), vector[dim]"`).
- **Combinators have no registry at all.** They live only in the grammar and
  in `check_kexpr`.
- **Built-in kernel settings live in two disagreeing lists.**
  - `lapsampler::builtin_settings` (private) is what the checker accepts.
  - `runner::BuiltIn` also lists `target_accept` and `adapt` for `rwm`, `mala`
    and `hmc`, for CSV provenance.
  - Phase 3 must take the checker's list.

All of this is Phase 3.1 input (`builtins()`, `combinators()`).

### 0.2.4 Column units

- `lsp-integration.md` says "columns count characters". I measured it with a
  probe that calls the real API, putting a prefix on the error's line:

  | Prefix | bytes / chars / UTF-16 | Reported col (base col 6, 9 or 8) |
  | --- | --- | --- |
  | `/*é*/` | 6 / 5 / 5 | 11 (+5) |
  | `/*𝜎*/` | 8 / 5 / 6 | 11 (+5) |
  | `/*𝜎𝜎𝜎*/` | 16 / 7 / 10 | 13 (+7) |
  | `/*𝜎𝜎*/` before a **pest syntax error** | 12 / 6 / 8 | 15 (+6) |
  | `/*𝜎𝜎*/` before a **`.lapsampler`** error | 12 / 6 / 8 | 14 (+6) |

- **Conclusion: 1-based Unicode scalar values, on every path** (checker, pest
  syntax errors, sampler).
- The LSP needs `utf16_col = sum of len_utf16() over the first (col-1) chars of
  the line`, then subtracts 1. Phase 2 tests must use an astral character such
  as `𝜎`; `é` alone would not catch a chars/UTF-16 mix-up.
- **Positions are points, not ranges.** The LSP must choose a range: the
  identifier or token at that point, else to end of line.
- `0:0` means the whole file.

## 0.3 The dependency question

**Finding: `laplace-sample` is not usable as a library dependency of
`laplace-lsp`, for two independent reasons.**

1. **libclang at build time.**
   - `laplace-sample` depends on `bridgestan = "=2.9.0"` unconditionally.
   - `bridgestan-2.9.0/build.rs` always runs `bindgen::Builder::…generate()` on
     `src/bridgestan.h`, and `bindgen 0.72` is a build-dependency. bindgen
     needs libclang.
   - `lsp-integration.md`'s "no C++ toolchain" is true at **run** time, but not
     at build time.
   - It also pulls in `nalgebra`, `faer`/`nano-gemm` (through nuts-rs), `rand`
     and `rand_distr`.
2. **It does not compile outside its own workspace.**
   - `nuts-rs = "=0.19.1"` is redirected to the vendored copy by
     `[patch.crates-io]` in the **`laplace` workspace root**.
   - Cargo applies `[patch]` only from the top-level workspace being built. A
     path or git dependency from `laplace-lsp` therefore gets upstream nuts-rs,
     which lacks `refresh_position`. Reproduced with a probe crate holding just
     `laplace-sample = { path = … }`:
     ```
     error[E0599]: no method named `refresh_position` found for mutable reference `&mut NutsChain<…>`
        --> laplace-sample/src/kernels/nuts.rs:247:19
     ```
   - Adding the same `[patch]` to the consumer makes it build. Every downstream
     crate would have to copy it, and it does nothing about libclang.

**What the editor path actually touches.** `editor.rs` uses:

- `lapkernel::{parse, check, builtins::Registry}`
- `lapsampler::SamplerFile::from_source_in`
- `libraries::Project`

The check logic is model-free, but the **types are tangled with the runtime**:

- `builtins::Entry` stores runtime `CallFn`/`GetFn` over `Ctx { state: ChainState,
  ev: Evaluator, rng: ChainRng }`. `state.rs` imports `target::Target`, which
  imports `bridgestan`.
- `lapsampler/mod.rs` imports `runner`, `target`, `kernels::nuts`, `csv`,
  `adapt` and `combinators`. `SamplerFile` stores `Arc<LapKernelFile>`, from
  `lapkernel::kernel`, which imports `runner` and `target`.
- `libraries.rs` needs only the `laplace` compiler crate (pure Rust; the LSP
  already depends on it).

**Recommended fix (option A): a `laplace-sampler-syntax` crate in the
`laplace` workspace.**

- **It holds:**
  - `lapkernel::{ast, parse, check, types, doc}` and the grammar;
  - a **static** registry: name, doc, type rule, `needs_chain`, `terminal`;
  - `lapsampler::parse` and the model-free check (today the first half of
    `from_source_in` plus `check_kexpr`);
  - `libraries` and `editor`.
- **Its dependencies:** `pest`, `pest_derive`, `nalgebra` (pure Rust, used by
  `types::Value`; it could move out later) and `laplace`. No bridgestan, no
  nuts-rs, no `[patch]`.
- **`laplace-sample` depends on it** and keeps the runtime parts:
  - the `CallFn`/`GetFn` table, keyed by name and attached to the static entries;
  - `InterpretedKernel`;
  - `SamplerFile::instantiate`/`build`.
- **`laplace_sample::editor` stays as a re-export,** so
  `lsp-integration.md` and `laplace sampler check` don't change.
- **Payoff:** Phase 3.1's `builtins()`, `combinators()` and `kernel_index()`
  have a natural home here.
- **Cost:** a real refactor on the `laplace` side, on its own branch
  `editor-api/syntax-crate`.
  - The registry has to be split in two.
  - `SamplerFile` has to separate "checked" from "built".
  - Roughly 3–4k lines move, with little logic change.
  - Its acceptance check: all existing `laplace-sample` tests pass unchanged,
    and `cargo build -p laplace-sampler-syntax` works with
    `LIBCLANG_PATH=/nonexistent` in a container without clang.

Alternatives considered:

- **(B) A `runtime` cargo feature on `laplace-sample`** (default on), making
  `bridgestan`, `nuts-rs` and `rand*` optional, with the editor using
  `default-features = false`.
  - Fewer moved files, but `#[cfg]` would run through `builtins.rs`, `state`,
    `kernel` and `lapsampler`.
  - The binary needs `required-features`.
  - Easy to break silently, because CI would need a second feature matrix.
  - Workable fallback if A is judged too big.
- **(C) Shell out to `laplace sampler check`.**
  - It needs the `laplace-sample` binary installed and saved files (the editor
    has unsaved text).
  - It costs a process per keystroke batch.
  - Rejected: it breaks "diagnostics come from `laplace_sample::editor`" in
    spirit and cannot check unsaved buffers.

Related: once the LSP takes `laplace-sampler-syntax` by git, it should take
`laplace` **from the same git revision**. Keeping today's
`path = "../../laplace"` alongside it would compile two different `laplace`
crates. The 0.3.0 `LockedPackage { kind }` break above shows how the path
dependency silently tracks whatever is checked out. See Q3.

## Does `.laplace` support have regression tests?

- **Highlighting: yes.**
  - 5 snapshot fixtures: `model.laplace`, `stats.laplacelib`,
    `wrapped.laplacelib`, and the patch-1 pair.
  - Any change to a shared repository rule that changes `.laplace` tokens fails
    `npm run test:grammar`.
  - New grammars that only `include` `source.laplace#…` can't change `.laplace`
    output, but the snapshots will prove it.
- **Extension behaviour: partly.**
  - The manifest test checks commands and settings, not languages, so adding
    languages won't break it.
  - The language-configuration test checks the `wordPattern`.
- **LSP behaviour: yes in principle (63 tests), but they don't compile today.**
  Fix that first (Q1), or Phase 2.4 ("`.laplace` tests unchanged") has no
  baseline.
- **No end-to-end JSON-RPC tests** and no CI. Phase 2.1 adds the former and
  Phase 4 the latter.

## Changes to existing code

None. New files:

- `docs/editor/phase-0-report.md`
- `docs/editor/open-questions.md`

## Decisions made during the phase

- Probes ran in a scratch crate outside both repos, so nothing in `laplace`
  was touched before approval.
- I did **not** fix the broken LSP tests or the rustfmt drift. Both are outside
  Phase 0's "no feature work". Proposed handling is in Q1.

## Open questions for Metodi

See `docs/editor/open-questions.md`:

- **Q1**: fix the pre-existing LSP test break and rustfmt drift first.
- **Q2**: the dependency fix, A vs B.
- **Q3**: how to pin `laplace`.
- **Q4**: grammar reuse where the token sets differ.
- **Q5**: `broken_mala.lapkernel` location and `.lapsampler` fixtures.

## Risks or plan changes for the next phases

- **Phase 2.1 shrinks:** the startup bug is fixed, so only the integration test
  is left to add.
- **Phase 2.2 is blocked on the `laplace`-side refactor (Q2).** Phase 1 is
  extension-only and is not blocked, so the refactor can run in parallel.
- **The plan asks for a CI job without libclang.** There is no CI in
  `laplace_tools` yet. Phase 2.2's check can be a local
  `docker run rust:slim cargo build` until Phase 4 adds CI.
- **Phase 3.4, go-to-definition for built-ins:** only `mala` and `rwm` ship a
  `.lapkernel` twin (`laplace-sample/kernels/`). `nuts` and `hmc` are
  Rust-only, so there is nothing to jump to. Show hover docs instead.
- **Phase 1.3, snippets:** the `laplace new kernel` templates are
  `authoring::kernel_skeleton` (`KernelKind::Proposal`/`Direct`). Copy them from
  there.
