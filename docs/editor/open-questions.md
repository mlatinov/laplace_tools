# Editor support for kernels: open questions

## Open

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


## Answered

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
