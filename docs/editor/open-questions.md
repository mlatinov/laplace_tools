# Editor support for kernels: open questions

## Open

None.

## Answered

### Q8: Completing parameter names inside `block(...)` (from Phase 3)

Inside `block(...)`, the names to offer are the model's parameters. A
`.lapsampler` doesn't say which model it's for: the model is given on the
command line (`laplace sample model.laplace --sampler s.lapsampler`).
Phase 3 doesn't build this, as the plan says.

Options:

- **(a) A sibling model.** Use `<stem>.laplace` next to the sampler, or the
  only `.laplace` in its directory. Read its `parameters { }` block with the
  `laplace` compiler crate's parser, which needs no BridgeStan. That gives
  names only: no unconstrained sizes, so no coverage check.
- **(b) A comment directive**, such as `// model: ../model.laplace`, read only
  by the editor.
- **(c) A workspace setting**, `laplace.samplerModel`, a glob from sampler
  to model.
- **(d) Nothing:** keep typing names by hand. `laplace sampler check
  --model` checks them.

**Recommendation: (a), with (b) as the override when (a) finds nothing or
the wrong file.** Both are editor-only and add no syntax to the language.
It is a Phase 3-sized follow-up; it needs no change in the `laplace` repo
beyond what the compiler crate already exposes. That last part is still to
be confirmed.

**Answered 2026-11-24: as recommended, (a) with (b) as the override.**
Built in Phase 4 (step 4.2), in the LSP only. Confirmed: the compiler
crate's `find_top_level_blocks` and `declarations` are enough, so no
`laplace` change is needed. Names that a template, macro or package call
adds to `parameters { }` aren't written there, so they aren't offered.

### Q9: What `kernel_index` scans when there is no `laplace.lock` (from Phase 3)

`kernel_index` lists the local `.lapkernel` files under the project
directory, up to 8 levels down. With no `laplace.lock` at or above the
sampler, the "project" is the sampler's own directory. A sampler saved
somewhere large without a lock is slow to index (measured, release build):

| Sampler's directory | First build | Repeated build |
|---|---|---|
| a real project (`laplace add langevin --path …`) | 6 ms | 2 ms |
| `laplace-sample/tests/targets` | 0.05 ms | 0.04 ms |
| `~/Data_Science_Projects` (no lock) | 0.55 s | 0.19 s |
| `~` (no lock) | 1.35 s | 0.30 s |

The LSP reuses an index for 10 seconds and drops it when a `.lapkernel`
is saved. Completion stays responsive, but the first request in such a
directory is slow.

Options (a `laplace`-repo change, in `laplace-sampler-syntax`):

- **(a)** Without a lock, scan only the sampler's directory and its
  `kernels/` subdirectory, not the whole tree.
- **(b)** Keep the tree scan but stop after N directories or files.
- **(c)** Leave it.

**Recommendation: (a).** A sampler outside a project can still name kernels
by path. Completion would offer only the nearby ones, but the files a
project uses are found as now, because projects have a lock. It's a small
change for the next `laplace` editor-API PR.

**Answered 2026-11-24: (a).** Built in laplace `05920aa`, on
`editor-api/symbols` (PR #8). Without a lock, an index of `~` takes
0.25 ms, down from 1.35 s. The LSP pin moved to it in Phase 4 (step 4.1).
The 10 s cache in the LSP stays: in a large project it still saves the
rescan on every keystroke.

### Q7: Moving the laplace pin from the PR branch to `main`, then to `v0.3.0` (from Phase 2)

`laplace-lsp` takes `laplace` and `laplace-sampler-syntax` from
`github.com/mlatinov/laplace` at `rev = 1dd1b69`. That's the tip of
`editor-api/syntax-crate`, [laplace PR #7](https://github.com/mlatinov/laplace/pull/7),
which isn't merged yet.

Steps, in order:

1. Merge laplace PR #7.
2. Change both `rev`s in `laplace-lsp/Cargo.toml` to the merge commit on
   `main`.
3. When the `v0.3.0` tag exists, use `tag = "v0.3.0"` instead.

Step 2 is a one-line change plus `cargo update -p laplace -p
laplace-sampler-syntax`. Phase 4's merge plan records the order.

**Recommendation:** do step 2 as the first commit after PR #7 merges, and
step 3 in Phase 4. Until then the pin is valid, because the branch is
pushed. Don't delete that branch until the pin has moved.

**Answered 2026-11-24: as recommended.**
- PR #7 isn't merged yet, so step 2 waits for it.
- Phase 3 moved the pin to the tip of `editor-api/symbols`
  ([laplace PR #8](https://github.com/mlatinov/laplace/pull/8), stacked on
  #7), which Phase 3 needs.
- Once #7 and #8 are merged, the first commit afterwards moves the pin to
  the merge commit on `main`; Phase 4 moves it to `v0.3.0`.
- Neither branch is deleted before the pin moves.

### Q6: `.laplace` doc-comment text is not scoped as a comment (from Phase 1)

The `.laplace` grammar's `comments` rule matches `// @param` but not the
text after it. In `// @param x the vector to standardise`, the text is
unscoped, and `vector` is coloured as a type. You can see it in
`vscode-laplace/fixtures/stats.laplacelib.snap`. The new kernel grammars
don't include this rule, so they're not affected.

Options:

- **(a)** A small separate PR that scopes the whole doc line as a comment,
  with the tag highlighted inside, as `lapkernel.tmLanguage.json` does. The
  `.laplace` snapshots change, and the diff is reviewed.
- **(b)** Leave it.

**Recommendation: (a), but outside this plan.** "Changes to `.laplace`
language features" is out of scope here, and it is a highlighting-only fix.

**Answered 2026-11-24: (a),** outside the plan:
[laplace_tools PR #6](https://github.com/mlatinov/laplace_tools/pull/6),
branch `laplace/doc-comment-scope`. Only the tag is highlighted: the
`@param` name is not, because `stats_patch1` writes `//@param vector t :`.
Only doc lines change in the four affected snapshots.

### Q1: Fix the pre-existing `laplace-lsp` breakage first (from Phase 0)

On `main` against `laplace` 0.3.0:

- `cargo test` doesn't compile: `LockedPackage` gained a `kind` field, with 5
  initializers in test code (`diagnostics.rs`, `stanc.rs`).
- `cargo fmt --check` fails in 10 of 11 files.

Options:

- **(a)** Two small commits at the start of Phase 2, before any feature work:
  first "add `kind` to test fixtures", then "cargo fmt" (format-only, nothing
  else).
- **(b)** A separate PR now, before Phase 1.
- **(c)** Leave fmt alone and fix only the tests.

**Recommendation: (b).** It's independent of the kernel work. It gives Phase 2.4
a green baseline to compare against, and keeps the fmt-only diff out of feature
review.

**Answered 2026-11-24: (b).** Fixed in PR #5 (`lsp/fix-tests-fmt`): commit `ceac6d1` adds `kind` to the test lockfiles (63/63 pass), and `eae8968` is the fmt-only commit.

### Q2: How `laplace-lsp` gets the sampler checker without libclang (from Phase 0)

`laplace-sample` can't be a dependency:

- `bridgestan`'s build script always runs bindgen, which needs libclang.
- The vendored nuts-rs `[patch]` doesn't apply outside the `laplace` workspace,
  so the crate doesn't even compile (`refresh_position` missing).

Options:

- **(A)** A new crate `laplace-sampler-syntax` in the `laplace` workspace with
  the parsers, the model-free checks, a static built-in registry, `libraries`
  and `editor`. `laplace-sample` depends on it and re-exports `editor`.
- **(B)** A default `runtime` feature on `laplace-sample` gating `bridgestan`,
  `nuts-rs` and `rand*`; the LSP uses `default-features = false`.
- **(C)** Shell out to `laplace sampler check` (rejected: unsaved buffers,
  needs the binary).

**Recommendation: (A)**, on `laplace` branch `editor-api/syntax-crate`,
reported and merged separately. It is also where Phase 3.1's `builtins()`,
`combinators()` and `kernel_index()` belong.

**Answered 2026-11-24: (A),** the `laplace-sampler-syntax` crate, on `laplace` branch `editor-api/syntax-crate`. Needed by Phase 2.2.

**Done in Phase 2:** [laplace PR #7](https://github.com/mlatinov/laplace/pull/7)
(open). One correction to Phase 0. The crate needs no libclang and no C++,
but building it does need a **C compiler**: `pest` depends on
`pest_stack_guard`, which uses `psm`, which assembles one file through `cc`.
Rust already needs that toolchain to link on Linux and macOS, so nothing
extra gets installed. But "no C toolchain at all" was wrong.

### Q3: Pinning `laplace` in `laplace-lsp` (from Phase 0)

Today it's `path = "../../laplace"`, the working tree. Phase 2.2 adds a git
dependency on `mlatinov/laplace` for the syntax crate.

Options:

- **(a)** Switch `laplace` to the same git `rev` at the same time.
- **(b)** Keep the path dependency for `laplace` (two copies of the crate in
  the graph).

**Recommendation: (a)**, with a `[patch."https://github.com/mlatinov/laplace"]`
path override documented for local co-development. Switch both to the
`v0.3.0` tag once it exists (Phase 4).

**Answered 2026-11-24: (a).** Pin both to the same git `rev`, with a documented `[patch]` path override for local work. Done in Phase 2.2.

### Q4: Grammar reuse where the token sets differ (from Phase 0, for 1.2)

The plan says to reuse `.laplace` scopes by `include`. Some `.laplace`
repository rules are wider than the kernel language:

- `type-keywords` also has `array`, `simplex`, `row_vector`, `complex*` and
  more;
- `control-keywords` also has `break`, `continue` and `target`;
- `constants` colours `pi` and `e` as constants, but in `.lapkernel` they are
  plain identifiers.

Options:

- **(a)** Include them anyway (simplest; the diagnostics still catch invalid use).
- **(b)** Include the rules that match exactly, and write narrow local rules
  where the sets differ. Use the same scope names (`storage.type.laplace`,
  …), so themes colour them identically.

**Recommendation: (b).** Include `comments`, `operators` and `function-call`;
write local `types`, `control` and `numbers` rules. Don't split rules in the
`.laplace` grammar to make them includable; that risks `.laplace` regressions
for little gain.

**Answered 2026-11-24: (b).** Done in Phase 1.2.

### Q5: Fixture locations (from Phase 0, for 1.2)

- `broken_mala.lapkernel` is at
  `laplace/laplace-sample/tests/fixtures/broken_mala.lapkernel`, not in
  `laplace-langevin`.
- `laplace-langevin` has `kernels/{mala,barker}.lapkernel` and no
  `.lapsampler`.
- The `.lapsampler` fixtures are the four files in
  `laplace/laplace-sample/tests/targets/`.

**Recommendation:** copy all of them into `vscode-laplace/fixtures/sampler/`,
so the snapshot tests don't depend on sibling repos, and note the source
commit in a README there.

**Answered 2026-11-24:** copy the fixtures into `vscode-laplace/fixtures/sampler/`. Done in Phase 1.2.
