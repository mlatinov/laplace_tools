# Phase 3 report (hover, completion, go-to-definition, signature help)

Branch `editor/phase-3`, from `editor/phase-2`. Nothing is published, tagged
or merged.

`.lapkernel` and `.lapsampler` files now have hover, completion,
go-to-definition and signature help. Everything shown comes from three new
editor-API functions in the `laplace` repo. Each reads the same table the
checker uses, and tests guard against drift.

## Done

| Step | Commit | What |
|---|---|---|
| 3.1 | `81a427f` | LSP pinned to laplace PR #8 (the new API). Q7 answered; Q8 opened |
| 3.2 | `8e85878` | Hover, both languages; the lexical cursor module |
| 3.3 | `be2bc86` | Completion, both languages |
| 3.4 | `139b859` | Go-to-definition from a `.lapsampler` kernel reference |
| 3.5 | `bf10cbb` | Signature help for combinators and kernel calls |
| (extra) | `a710e76` | The kernel index is reused for 10 s per sampler directory, and dropped when a kernel is saved; Q9 |
| docs | this commit | This report; README section |

## Changes in the `laplace` repo (separate merge)

Branch `editor-api/symbols`:
**[laplace PR #8](https://github.com/mlatinov/laplace/pull/8)**, stacked on
PR #7 (its base is `editor-api/syntax-crate`; retarget it to `main` after #7
merges). It is open and not merged. The LSP is pinned to its tip, `c6d43bf`.

| Commit | What |
|---|---|
| `b2b53a1` | Registry: each built-in gets `signatures` (`real log_density(vector[dim] x)`). A test feeds every signature's argument types to the type rule and checks the declared return type. I tried three wrong signatures, and each one fails the test |
| `a763218` | `builtins()`, `combinators()`, `kernel_index(project, from)`, and the moves needed so nothing exists twice (below) |
| `c6d43bf` | `lsp-integration.md` documents the three functions |

What `a763218` moved or added:

- **One list of built-in kernel settings.** The checker's per-built-in
  settings and `laplace sampler doc`/`list` used to each keep their own copy.
  Both now come from `BuiltIn` (`sampler_settings`, `adaptation`, `line`,
  `doc`, plus a new `brief`).
- **`laplace sampler doc` moved into the syntax crate.** The CLI's `doc` and
  `list` output is byte-identical before and after; I compared six
  commands.
- **`BuiltIn::source()`** embeds the built-in kernels' Rust source. The
  provenance hash reads it from there, unchanged.
- **Path dependencies:** `KernelPackage.source_dir` gives a path dependency's
  own directory. That's the file to open, not the cache copy each resolve
  overwrites. `laplace::resolve` re-exports `parse_path_source` for it.
- **Crate docs corrected:** a C compiler is needed (pest → psm → cc); no
  libclang or C++.

Checks: `cargo test --workspace` gives 1013 passed, 0 failed; fmt and clippy
are clean at each commit.

The plan's 3.1 check, on a real project: I created a scratch project and ran
`laplace add langevin --path ~/Data_Science_Projects/laplace-langevin`.
`kernel_index` then lists:
- the 4 built-ins;
- `langevin::barker` and `langevin::mala`, with files in
  `laplace-langevin/kernels`, their tuning values, `@adapt` tags, and
  `propose`.

**Merge order:** #7, then #8 (retargeted to `main`), then move the LSP pin to
the merge commit (Q7). Don't delete either branch before the pin moves.

## What each feature does

**`.lapkernel`:**

- **Hover on a built-in** shows its signatures, its doc, the random numbers
  it draws, and whether a tuning default may use it.
- **Completion** offers every built-in in any block of the kernel. In
  `tuning { }` it offers only the ones a default may use, because the others
  need a running chain. Functions are inserted as `name(…)`.

**`.lapsampler`** (inside `kernel { }`):

- **Hover:**
  - on a kernel reference (a built-in, `langevin::mala`, or
    `./kernels/x.lapkernel`), the text `laplace sampler doc` prints. A kernel
    that doesn't check shows its error;
  - on a combinator, its signature and doc;
  - on a setting inside a call, its declaration, `@adapt` tag and
    `//@param` text.
- **Completion:**
  - at the start of a line: `block(…) :` and `rest :`;
  - wherever a kernel goes (after the colon, inside `seq`, after a `mix`
    weight, `repeat`'s second argument, `adapt`'s first): every kernel, then
    `seq`/`mix`/`repeat`/`adapt` (not `on`, which a kernel line can't
    contain);
  - inside a kernel's parentheses: its settings not yet given, with the
    default as placeholder;
  - inside `adapt(…, `: `target_accept`;
  - the completion replaces what's typed so far, including `langevin:` and
    `./ker`.
- **Go to definition:**
  - a package kernel opens its `.lapkernel` at the kernel's name. For a path
    dependency, that's the file in its own directory;
  - a local file opens directly;
  - a built-in opens its Rust source. The source is written read-only to
    `~/.cache/laplace-lsp/builtin-kernels/<hash>/<name>.rs` (or under
    `$XDG_CACHE_HOME`).
- **Signature help:**
  - for combinators, the argument the cursor is in is active;
  - for kernel calls, the label is `langevin::mala(eps = 1, m = rep_vector(1,
    dim))`. Settings are named, so the active one is the one being written;
    otherwise it's the first not yet given.
- **Not built (Q8):** parameter names inside `block(…)`. They need the model,
  which a `.lapsampler` doesn't name.

**How the cursor is read.** The text is usually mid-edit and doesn't parse,
so `sampler/cursor.rs` works lexically:
- comments are blanked out, keeping offsets;
- the enclosing section is the first word before each open `{`;
- the call, its argument index, and the kernel line come from counting
  brackets and commas.

Nothing in a comment gets hover, completion, a definition or a signature.

## Checks

- **At every commit** (in a separate worktree): `cargo fmt --check`, clippy
  `-D warnings`, `cargo test`, and the extension's `npm test`. All pass. At
  `a710e76`: 131 Rust tests (123 unit + 8 stdio) and 62 extension tests.
- **Feature tests:**
  - **Setup:** they run in a temporary project with a path-dependency
    package (the `laplace-langevin` fixtures plus a broken kernel) and a
    temporary package cache, so nothing touches `~/.laplace`.
  - **What they assert:** exact labels, inserts, replaced ranges, details,
    order, hover text and ranges, definition targets, and signature labels
    and active parameters.
  - **Mutation checks:** I broke five completion rules one at a time: the
    tuning filter, the argument-index rule, the given-settings filter, the
    `on` exclusion, and the comment check. Each one fails a test.
- **Over stdio** (the real binary), one test each for:
  - hover in both languages;
  - completion, with `textEdit` snippets;
  - a `(` trigger in a `.laplace` file answering nothing;
  - definition of a local kernel, and of a built-in written under a
    temporary `XDG_CACHE_HOME`;
  - signature help, with its UTF-16 label offsets;
  - no signature help for `.laplace`;
  - the index being refreshed by saving a kernel.
- **`.laplace` regression:** comparing test names against the end of Phase 2,
  none were removed and 40 were added.
  - The new completion trigger characters (`(`, `,`, `/`) reach `.laplace`
    files too, but a request they trigger there answers nothing. That's what
    the user saw before, when no request was made.
  - Signature help answers nothing for `.laplace`, which had none before.
- **Smoke test of the release binary** on the real `laplace-langevin` project:
  - hover shows the CLI's doc text;
  - definition opens
    `~/Data_Science_Projects/laplace-langevin/kernels/mala.lapkernel` at line
    14, `mala`;
  - completion and signature help in `langevin::mala(` list `eps` and `m`;
  - `nuts` opens its Rust source.

## Decisions made during the phase

1. **Signatures went into the registry.** The alternative was parsing them
   out of the doc strings. A test now ties each signature to its type rule,
   which is what "taken from the existing registry so they can't drift"
   needs. The function docs no longer repeat the signature; nothing else
   read them.
2. **The definition of a built-in is its Rust implementation.** The plan
   says "the file shipped with laplace-sample". Only `mala` and `rwm` have
   `.lapkernel` twins there, while `nuts` and `hmc` exist only in Rust, and
   the editor has no checkout of `laplace-sample`. The server therefore
   writes the source built into it, read-only, to a cache file. If you'd
   rather `mala`/`rwm` open their `.lapkernel` twins, that's a small change.
3. **A path dependency opens its own file.** Opening the cache copy would let
   you edit a file that the next `laplace` command overwrites.
4. **`on` is never offered in completion,** but it still has hover and
   signature help, because a kernel line already is `on(block, kernel)`.
5. **New completion trigger characters, guarded for `.laplace`.** `(` and `,`
   bring up settings, and `/` brings up paths. In `.laplace` files they
   answer nothing.
6. **Each cursor helper arrives in the commit that first uses it,** so each
   commit is clean under clippy on its own.
7. **The index cache** (10 s, cleared on any `.lapkernel` save) is in the
   server, not the API. It's for completion latency (Q9).
8. **No signature help for `.lapkernel` built-in function calls.** The plan
   asks only for combinators and kernel calls, and hover already shows
   built-ins' signatures. Adding it would be about 20 lines on the same
   machinery.

## Open questions for Metodi

- **Q8: completing parameter names in `block(…)`.** Recommendation: read the
  sibling `<stem>.laplace`'s `parameters { }` block, with a
  `// model: …` comment as the override.
- **Q9: `kernel_index` without a lockfile.** It scans the sampler's
  directory tree, which takes up to 1.3 s in `~`. Recommendation: without a
  lock, scan only the sampler's directory and its `kernels/`.

Q7 is answered. Details for all three are in `open-questions.md`.

## Risks or plan changes for Phase 4

- **The pin moves twice before release:** to `main` after #7 and #8 merge,
  then to `v0.3.0`. Phase 4's merge plan has to order: laplace #7 → #8 → tag
  → pin → laplace_tools PRs.
- **Package resolution writes to the cache.** `kernel_index` resolves
  packages as `laplace sample` does, so a path dependency is re-synced into
  the cache, at most once per 10 s per sampler directory. That's the same
  behaviour as the diagnostics from Phase 2, which run on every edit.
- **The extension is unchanged** in this phase: the new features are server
  capabilities. The README in the installed VSIX is from Phase 2. Phase 4's
  package will carry the new one.

## Manual checklist for Metodi

First, in Positron: **Developer: Reload Window**. This restarts the release
server, which I've already rebuilt; the extension itself didn't change.

**Setup (once):** a project that uses `laplace-langevin` through a path
dependency.

```sh
mkdir -p ~/Data_Science_Projects/langevin-demo && cd ~/Data_Science_Projects/langevin-demo
printf 'name = "langevin-demo"\nversion = "0.1.0"\n' > laplace.toml
laplace add langevin --path ../laplace-langevin
mkdir -p kernels && cp ../laplace-langevin/kernels/mala.lapkernel kernels/mine.lapkernel
cat > demo.lapsampler <<'EOF'
sampler demo {
  library { import langevin }
  kernel {
    block(a) : adapt(langevin::mala(eps = 0.1))
    block(b) : seq(rwm(step_size = 0.5), ./kernels/mine.lapkernel)
    rest : nuts()
  }
}
EOF
```

Open the folder in Positron.

1. **`~/Data_Science_Projects/laplace-langevin/kernels/mala.lapkernel`**
   - [ ] Hovering `log_density_grad`, `std_normal_rng`, `grad` and `propose`
     shows each one's signature and doc. `std_normal_rng` also lists the
     random numbers it draws.
   - [ ] Hovering a local name (`q_new`), a keyword (`step`) or a comment
     shows nothing.
   - [ ] In `step { }`, typing `std_` offers `std_normal_rng` and
     `std_normal_lpdf`. Accepting one inserts `std_normal_rng()` with the
     cursor inside the parentheses.
   - [ ] In `tuning { }`, after `= `, Ctrl+Space offers `dim`, `rep_vector`
     and `exp`, but not `q`, `grad` or `std_normal_rng`.
2. **`langevin-demo/demo.lapsampler`**
   - [ ] **Hover** `langevin::mala`: the doc text with `eps` and `m`, their
     `@adapt` tags, and "ends in propose()". Then hover `rwm`, `nuts`, and
     `./kernels/mine.lapkernel`.
   - [ ] **Hover** `adapt` and `seq`: the signature and doc. Hover `eps` in
     `langevin::mala(eps = 0.1)`: its declaration, tag and `//@param` text.
   - [ ] **Completion:** at the start of a new line in `kernel { }`, type `bl`.
     It offers `block` and `rest`. Accept `block`, type a name, then Tab: the
     cursor lands after the `:`.
   - [ ] **Completion:** after `rest : `, Ctrl+Space lists the built-ins,
     `langevin::barker`, `langevin::mala`, `./kernels/mine.lapkernel`, then
     `seq`/`mix`/`repeat`/`adapt` (no `on`). Typing `langevin:` narrows the
     list to the package.
   - [ ] **Completion:** type `hmc(`. Settings appear on the `(`. Accepting
     `steps` inserts `steps = 10`, with `10` selected. After `, `, only the
     settings not yet given are offered.
   - [ ] **Go to definition** (F12) on `langevin::mala`: it opens
     `laplace-langevin/kernels/mala.lapkernel` (not a file under
     `~/.laplace`), with the cursor on `mala`.
   - [ ] **Go to definition** on `./kernels/mine.lapkernel`: it opens that
     file. On `nuts`: it opens `nuts.rs`, read-only (Positron should refuse
     edits or warn).
   - [ ] **Signature help:** type `langevin::mala(`. The popup shows
     `langevin::mala(eps = 1, m = rep_vector(1, dim))` with `eps` bold. Type
     `m` and `m` becomes bold. Inside `adapt(…, `, `target_accept = ...` is
     bold.
   - [ ] **Comments:** nothing pops up after `//` on any of these lines.
   - [ ] **Refresh:** create `kernels/new.lapkernel` (copy `mine`) and save it.
     `./kernels/new.lapkernel` is offered after `rest : ` straight away.
3. **`.laplace` regression**
   - [ ] In any `.laplace` model, typing `normal(` or `,` brings up no new
     popup. `pkg::` completion, hover and go-to-definition behave as before.

**STOP 3.** I'm waiting for your go (and answers to Q8 and Q9) before
Phase 4.
