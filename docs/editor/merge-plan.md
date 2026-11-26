# Merge plan: editor support for `.lapkernel` and `.lapsampler`

Two repos are involved, and their merges have an order. Every step here is
Metodi's to take: nothing has been merged, tagged or published.

## What is open

**`laplace`**

| PR | Branch → base | What | State |
|---|---|---|---|
| [#7](https://github.com/mlatinov/laplace/pull/7) | `editor-api/syntax-crate` → `main` | The `laplace-sampler-syntax` crate: the sampler parsers and model-free checks, buildable without libclang; CI for it | open; 3 commits ahead of `main`, 0 behind |
| [#8](https://github.com/mlatinov/laplace/pull/8) | `editor-api/symbols` → `editor-api/syntax-crate` | `builtins()`, `combinators()`, `kernel_index()`; built-in signatures in the registry; only nearby files indexed without a lock (Q9) | open; 4 commits on top of #7 |
| [#9](https://github.com/mlatinov/laplace/pull/9) | `sampler/kernel-math` → `editor-api/symbols` | The kernel math library (32 safe Stan functions, `docs/sampler/kernel-functions.md`); library names no longer reserved; deeper `propose`/`move` docs; `ADAPT_DOC`, `adapt_tags()` | open; 1 commit on top of #8 |

laplace PR #6 (`sampler/phase-3`) is unrelated: it's 36 commits behind
`main`, which already has the sampler work. It doesn't block anything here.

**`laplace_tools`**

| PR / branch | What | State |
|---|---|---|
| [#5](https://github.com/mlatinov/laplace_tools/pull/5) `lsp/fix-tests-fmt` | LSP test build fixed; `cargo fmt` | open; already merged into `editor/phase-2`, so in every later phase branch |
| [#6](https://github.com/mlatinov/laplace_tools/pull/6) `laplace/doc-comment-scope` | Q6: the whole doc-comment line is a comment in `.laplace` | open; independent of the phase branches. It merges cleanly with `editor/phase-4`, and the extension's tests pass on the result |
| `editor/phase-4` (no PR yet) | Phases 0–4, each stacked on the one before: languages, grammars, snippets, diagnostics, hover, completion, definition, signature help, versions, CI | 25 commits ahead of `main`, 0 behind; not pushed until Metodi says |

`laplace-lsp` depends on the `laplace` and `laplace-sampler-syntax` crates
by git, pinned to `be5cb46` (the tip of `sampler/kernel-math`, on top of
laplace #8). The branch is pushed, so fresh builds and CI can fetch it.
When #8 merges, open the `sampler/kernel-math` PR against `main` and merge
it before moving the pin to `main`.

```toml
laplace = { git = "https://github.com/mlatinov/laplace", rev = "be5cb4609780b00e4334b4b321c880e05cb7c66f" }
laplace-sampler-syntax = { git = "https://github.com/mlatinov/laplace", rev = "be5cb4609780b00e4334b4b321c880e05cb7c66f" }
```

## Order

### 1. `laplace`: #7, then #8

1. Merge **#7** into `main`.
2. **Retarget #8 to `main`.** GitHub may do this when #7's branch is deleted,
   but deleting it now would break the pin (step 3). So retarget by hand
   (`gh pr edit 8 --base main`) and keep both branches.
3. Merge **#8**.
4. Check: `cargo test --workspace` on `main`, and the `syntax-without-libclang`
   job in `sampler.yml`.

Merge commits or rebase merges both work. The pin moves to whatever commit
lands on `main`.

### 2. `laplace_tools`: pin to `laplace` `main`

On `editor/phase-4` (or a follow-up branch):

1. In `laplace-lsp/Cargo.toml`, set both `rev`s to #8's merge commit on
   `laplace` `main`.
2. `cargo update -p laplace -p laplace-sampler-syntax`, then fmt, clippy,
   `cargo test` and `npm test`.
3. One commit: "pin laplace to main <sha>".

After this, nothing depends on the `editor-api/*` branches, so they can be
deleted.

### 3. `laplace`: release 0.3.0 (Metodi)

`laplace`'s own `docs/sampler/merge-plan.md` ("Before tagging 0.3.0") wants
the editor update ready first (its Q17); step 2 above does that. Its other
items stand:
- a `## [0.3.0]` CHANGELOG section;
- the `laplace-sample` version (Q20);
- `release-sampler.yml`.

Then Metodi tags `v0.3.0`.

### 4. `laplace_tools`: switch the dependency to the `v0.3.0` tag

```toml
laplace = { git = "https://github.com/mlatinov/laplace", tag = "v0.3.0" }
laplace-sampler-syntax = { git = "https://github.com/mlatinov/laplace", tag = "v0.3.0" }
```

- Then `cargo update -p laplace -p laplace-sampler-syntax` and the checks.
- `Cargo.lock` records the tag's commit. CI reads the commit from there for
  the `laplace` CLI it installs, so the workflow doesn't change.
- If `v0.3.0` points at the same commit as step 2, this changes nothing but
  the spelling. If it doesn't, the editor's behaviour moves with it, so rerun
  the Phase 3 and 4 manual checklists.
- Don't depend on crates.io for these: `laplace-sampler-syntax` isn't
  published there, and publishing it is a separate decision.

### 5. `laplace_tools`: merge

1. **#5.** Its two commits are already in `editor/phase-4`, so after #5
   merges, the phase PR no longer shows them.
2. **#6**, before or after the phase PR; they don't conflict.
3. **A PR from `editor/phase-4` to `main`.** It holds phases 0–4. Merge with
   a merge commit, not a squash, so the per-step commits each phase report
   refers to stay in history.
4. CI (`.github/workflows/editor.yml`) must be green on the PR:
   - `lsp`: fmt, clippy, tests;
   - `extension`: type check, unit, grammar, bundle and snippet tests
     against the real CLI, then the VSIX build.

The `editor/phase-0` … `editor/phase-3` branches are contained in
`editor/phase-4`, so they need no PRs of their own.

## Before publishing to Open VSX (Metodi)

None of this is done; publishing is Metodi's call.

- [ ] Steps 1–5 are done. `main` is pinned to the `v0.3.0` tag, not a
      branch commit.
- [ ] `CHANGELOG.md` in `vscode-laplace` and `laplace-lsp`: change
      "(unreleased)" to the date.
- [ ] **A LICENSE file.** `package.json` says MIT, but there's no LICENSE
      file, and `vsce package` warns about it. Open VSX shows the license
      from the file; add one (and to `laplace-lsp`).
- [ ] **`repository` in `package.json`** (`vsce` warns): the
      `laplace_tools` URL.
- [ ] **`laplace-lsp` 0.2.0 has to reach users.** The extension doesn't
      bundle the server; the README tells users to build it
      (`cargo install --path laplace-lsp`, or `cargo install --git
      https://github.com/mlatinov/laplace_tools laplace-lsp`). With an old
      0.1.0 server, sampler files get highlighting and snippets only, as the
      README says. Shipping server binaries is a separate decision.
- [ ] Build the VSIX from `main`: either CI's `vsix` artifact or `npm run
      package`. Install it in VS Code and in Positron, and run the Phase 3 and
      Phase 4 manual checklists once more on the installed copy.
- [ ] `.laplace` regression check on a real project: highlighting, coloring,
      `pkg::` completion, hover with `//@math`, diagnostics.
- [ ] The Open VSX namespace `laplace-lang` and its token (outside this
      plan's scope).
- [ ] Then `npx ovsx publish laplace-lang-0.5.0.vsix`, and tag
      `laplace_tools` if wanted.

## If something goes wrong

- **#8 needs changes after review:** push them to `editor-api/symbols`, then
  move the `laplace_tools` pin to the new tip. A pin to an unmerged commit is
  fine while the branch exists.
- **The tag isn't wanted yet:** stay on the `main` commit (step 2) and
  publish from that. Cargo builds the same either way. The tag only makes the
  dependency readable.
- **A CI run fails only in the snippet tests:** the CLI it built is the
  pinned commit's. Check that `laplace new kernel` at that commit still
  writes what the `kernel` snippet does.
