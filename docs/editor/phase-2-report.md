# Phase 2 report (live diagnostics through the LSP)

Branch `editor/phase-2`, from `editor/phase-1` with `lsp/fix-tests-fmt` (PR #5)
merged in for a green baseline. Nothing is published, tagged or merged.

`.lapkernel` and `.lapsampler` files now show errors as you type. The errors
come from `laplace_sampler_syntax::editor`, which is the code behind
`laplace sampler check`. The server only places them: it converts positions,
picks a range and links the related files. It checks nothing itself.

## Done

| Step | Commit | What |
|---|---|---|
| merge | `f15fa64` | `lsp/fix-tests-fmt` (PR #5): the LSP tests build against laplace 0.3.0 |
| 2.1 | `de51fc8` | End-to-end stdio test: `initialize` / `initialized` / `shutdown` / `exit`, and a test that the server waits when given no input |
| 2.1 fix | `52b515e` | That test allows the async "initialized" log notification to arrive late (it was flaky, about 1 run in 6) |
| 2.2 | `8913c26` | `laplace` and `laplace-sampler-syntax` as git dependencies at one pinned rev; a documented `[patch]` override for local work |
| 2.3 | `b009d17` | Diagnostics for both languages (server + client), README section, tests |
| 2.3 | `655d32f` | Two more cases the plan names: missing `target_accept`, a terminal missing on one path |
| 2.4 | (no commit) | The `.laplace` tests are unchanged and pass (below) |
| docs | this commit | This report; `open-questions.md`: Q6 answered, Q2 C-compiler correction, new Q7 |

## Changes in the `laplace` repo (separate merge)

Branch `editor-api/syntax-crate`, **[laplace PR #7](https://github.com/mlatinov/laplace/pull/7)**.
It is open and not merged. The LSP is pinned to its tip, `1dd1b69`.

| Commit | What |
|---|---|
| `675eeb8` | New crate `laplace-sampler-syntax`: grammars, syntax trees, static checks, built-in registry, `BuiltIn` / `SamplerSpec`, `LapKernelFile`, kernel docs, package lookup, `editor`. `laplace-sample` builds on it and re-exports every moved module at its old path. What runs stays in `laplace-sample`: `natives` (built-in implementations, with a test that they match the registry), `BuildKernel`, `SamplerRuntime` |
| `a30afe8` | `sampler.yml`: clippy and tests for the new crate, plus a `syntax-without-libclang` job in `rust:1-slim` that first checks there is no libclang |
| `1dd1b69` | `docs/sampler/lsp-integration.md`: which crate an editor depends on, and why not `laplace-sample`; what building it needs; the column unit |

No behaviour changes. The test names are the same 193 plus one new
consistency test, and the editor doctest moved crates. `cargo test --workspace`
gives 998 passed, 0 failed, the same as before. fmt and clippy
(`--workspace --all-targets -D warnings`) are clean at each of the three
commits. The statistical/SBC suites (`--ignored`, which need BridgeStan) were
not run; the 4 bit-for-bit interpreter-equivalence tests pass.

**Merge order:** PR #7 first. Then move the LSP pin to the merge commit
(Q7). Don't delete the `editor-api/syntax-crate` branch before that: the pin
refers to it.

## Checks

**`laplace_tools`, at every commit above** (in a separate worktree):
`cargo fmt --check`, `cargo clippy --all-targets -D warnings` and
`cargo test` in `laplace-lsp`, and `npm test` in `vscode-laplace`. All pass.
At `655d32f`: 91 Rust tests (88 unit + 3 stdio) and 62 extension tests.

**2.1.** I re-ran the stdio test with `SkipLeadingWhitespace` removed from
`main.rs`. The "waits for input" test fails without it, so the test does
catch the original bug. 15 of 15 runs pass after the fix.

**2.2: no libclang.** I used `bwrap` to bind `/dev/null` over
`libclang.so.22.1.8`, `libclang-cpp.so.22.1` and `/usr/bin/clang-22`. Inside
that sandbox:

| Build | Result |
|---|---|
| `laplace-sample` (control) | fails in `bridgestan`'s build script, as expected |
| `laplace-sampler-syntax`, from a separate workspace with no `[patch]` | builds, checks a kernel |
| `laplace-lsp`, release, from an **empty** target dir, `--locked` | builds; all tests pass |

`cargo tree` for `laplace-lsp` has no bridgestan, bindgen, clang-sys or
nuts-rs. In CI, the `syntax-without-libclang` job does the same check on
every push, in the `laplace` repo.

**Correction to Phase 0:** building does need a **C compiler**. `pest`
depends on `pest_stack_guard`, which uses `psm`, which assembles one file
through `cc`. Rust already needs that toolchain to link on Linux and macOS,
so nothing extra gets installed, but "no C toolchain" was wrong. This is
recorded under Q2 and in `lsp-integration.md`.

**2.3: diagnostics.** Every case asserts the exact range (0-based, UTF-16)
and the exact message:

- **`.lapkernel`:**
  - syntax errors, mid-file and at EOF;
  - unknown names and functions;
  - no `step` block;
  - unknown `@adapt` tag, missing `target_accept`, `target_accept` out of range;
  - type errors: assignment, wrong `propose` arguments, `vector * vector`,
    density without `|`, unused return value;
  - terminals: none, missing on one path, inside a loop, `return` in a step;
  - assigning or redeclaring a built-in.
- **`.lapsampler`:**
  - syntax error;
  - unknown built-in kernel, unknown setting, mistyped setting;
  - missing local kernel;
  - local kernel that doesn't check: the multi-line message, plus related
    information that points at each of its errors in that file;
  - `adapt()` on a kernel with nothing to adapt;
  - `import` with no `laplace.lock`;
  - `import` with a lock in a parent directory, which is used as the project.
- **UTF-16:**
  - `/* 𝜎 */ nope`: the checker's column 36 (in chars) becomes 37 (UTF-16), and
    the test slices the line by UTF-16 units to confirm the range covers `nope`;
  - the same check in a `.lapsampler`, with two 𝜎s.
- **Zero diagnostics:**
  - the base kernel;
  - the four fixture kernels (`laplace-sample` mala and rwm, `laplace-langevin`
    mala and barker);
  - three valid samplers.
- **Over stdio:**
  - open a broken kernel, and the diagnostic arrives with the right range;
  - fix it, and the diagnostics clear;
  - break it again, then close it, and the diagnostics clear;
  - a kernel gets no semantic tokens.

**2.4.** I compared the test names from before and after 2.3. The 63
`.laplace` tests and the 2 stdio tests are all still present and pass. The
new tests are additions only. Requests for `.laplace` and `.laplacelib`
files take exactly the same path as before.

## How it works

- **Routing:**
  - `sampler::Language::for_uri` picks the `.lapkernel` and `.lapsampler`
    extensions. Everything else goes the existing `.laplace` route,
    unchanged.
  - For the sampler languages, completion, hover, definition and semantic
    tokens return nothing; those are `.laplace` features, and Phase 3 covers
    them.
- **Triggers:**
  - `didOpen`, `didChange` (debounced 200 ms; `.laplace` keeps its 350 ms)
    and `didSave`. The check runs in `spawn_blocking`, because samplers read
    kernel files from disk.
  - `didClose` clears diagnostics.
  - Saving a `.lapkernel` re-checks every open `.lapsampler`, since a sampler
    reads its kernels from disk.
- **Ranges:**
  - The checker reports a point. The server marks the token at that point:
    an identifier, `pkg::name`, `@adapt`, a number such as `-1.5E+1`, or a
    kernel path up to `(`. Anything else gets a single character.
  - A position past the end of a line marks the line's last character.
  - `0:0` means the start of the file.
- **Project:**
  - The nearest directory with `laplace.lock` at or above the sampler (where
    you would run `laplace sample`). If there isn't one, the sampler's own
    directory, so the checker's "no laplace.lock" message appears.
  - Packages come from `laplace::resolve::default_cache_root()`. It honours
    `LAPLACE_HOME`, unlike the LSP's older `HOME`-only helper, which the
    `.laplace` features still use.
- **Client:**
  - `src/languages.ts` holds `SERVER_LANGUAGES`, the single list behind the
    `documentSelector`.
  - A test keeps it equal to the contributed languages and to
    `activationEvents`.

## Decisions made during the phase

1. **Token ranges, not whole lines.** Whole lines are too coarse: in
   `vector[dim] q_new = q + nope * …`, the squiggle should sit under `nope`.
   The token rules are kept small, and anything they don't cover gets one
   character.
2. **The multi-line message is kept whole.** The first line is the summary,
   and the indented lines name the files. Each indented line is also
   attached as related information, which shows as a clickable location
   (only when that file exists on disk). A library kernel is named
   `<package>/kernels/x.lapkernel`, which is not an absolute path, so it
   gets no link. Its text is still in the message.
3. **Mixed-file diagnostics.** If the checker ever reports a diagnostic in
   another file (it doesn't today), the server puts it at the top of the
   file with its `file:line:col` in the message, rather than at a wrong
   position. This has a test.
4. **Fixing the flaky test in a new commit**, not by rewriting 2.1. The
   branch is local, but a separate commit keeps the history honest.
5. **The release binary and VSIX.** I rebuilt
   `laplace-lsp/target/release/laplace-lsp` (your `laplace.serverPath`), and
   repackaged and installed `laplace-lang-0.4.0.vsix` into Positron locally
   (`--force`), because the client's `documentSelector` changed. Nothing
   was published.

## Open questions for Metodi

- **Q7** (new): move the pin from the PR #7 branch to `main` after the merge,
  and to `v0.3.0` once that tag exists. **Recommendation:** do the first as
  the first commit after PR #7 merges, and the second in Phase 4.

Q6 is answered (PR #6). See `open-questions.md`.

## Risks or plan changes for the next phases

- **Phase 3.1 changes the same `laplace` crate.** `builtins()`,
  `combinators()` and `kernel_index()` belong in `laplace-sampler-syntax`,
  which now holds the registry. That branch (`editor-api/symbols`) should
  start from PR #7, or from `main` after it merges, and the pin moves again.
- **The unsaved state of other files.** A `.lapsampler` sees a referenced
  kernel's saved contents, not unsaved edits in another tab. That matches
  `laplace sampler check`, and saving the kernel re-checks the open samplers.
- **Two package-cache helpers.** The `.laplace` features still use
  `workspace::default_cache_root()`, which ignores `LAPLACE_HOME`. Unifying
  them would change `.laplace` behaviour, so it's out of scope for this plan.
  It's a one-line follow-up if you want it.

## Manual checklist for Metodi

First, in Positron: **Developer: Reload Window**. The new VSIX and the
release server are already installed. Open **View → Problems** next to the
editor.

1. **`~/Data_Science_Projects/laplace-langevin/kernels/mala.lapkernel`**: make
   each edit, check that the squiggle and the Problems entry appear within
   about a quarter-second, then undo and check that they clear.
   - [ ] **Type error:** change `vector[dim] q_new =` to `int q_new =`. The
     squiggle is under `q_new` ("`q_new` is int, but this is vector"), with a
     second one on `propose`.
   - [ ] **No terminal:** delete the `propose(...)` line. The squiggle is on
     the `{` of `step {` ("never calls propose()/move()").
   - [ ] **Terminal on one path:** wrap `propose(...)` in
     `if (eps > 1) { ... }`. Expect "some path through step { } ends without
     propose()/move()".
   - [ ] **Bad `@adapt` tag:** change `@adapt(metric)` to `@adapt(metrc)`. The
     squiggle is under `@adapt`.
   - [ ] **Missing `target_accept`:** change
     `@adapt(step_size, target_accept = 0.574)` to `@adapt(step_size)`.
     Expect "needs the kernel's default acceptance target".
   - [ ] **Unknown name after a 𝜎:** on the `mu_fwd` line, change `grad` to
     `/* 𝜎 */ nope`. The squiggle covers exactly `nope`.
   - [ ] **Syntax error:** delete a `;`. One squiggle, at the token after it.
   - [ ] Hovering a squiggle shows the source **laplace sampler**.
2. **`~/Data_Science_Projects/laplace/laplace-sample/tests/targets/hybrid_simplex.lapsampler`**
   - [ ] As opened, there are no problems. (Coverage isn't checked; the README
     says so.)
   - [ ] Change a built-in such as `nuts()` to `nutz()`. There's a squiggle
     under `nutz`, with the list of built-ins.
   - [ ] Change the kernel path to `./nope.lapkernel`. The whole path is
     underlined ("cannot read …").
   - [ ] Undo. Then open the kernel that line names
     (`laplace/laplace-sample/kernels/mala.lapkernel`, a tracked file:
     `git checkout` it afterwards), introduce an error and **save**. The sampler gets an error on the path, "… does not check:",
     and its related information links to the line in the kernel. Fix the
     kernel and save, and the sampler clears.
   - [ ] Close the sampler. Its entries leave the Problems panel.
3. **`.laplace` regression**
   - [ ] Open any `.laplace` model. Semantic colouring, `pkg::` completion,
     hover and live diagnostics behave as before.
4. **Optional, in a terminal:**
   `laplace sampler check <the same broken file>` prints the same messages
   the editor shows.

**STOP 2.** I'm waiting for your go before Phase 3. Phase 3 opens with
`editor-api/symbols` in the `laplace` repo; see Risks.
