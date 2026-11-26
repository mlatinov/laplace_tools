# Changelog

## 0.2.0 (unreleased)

### Added: `.lapkernel` and `.lapsampler` files, experimental

- Diagnostics, from `laplace_sampler_syntax::editor`. That's the same code
  as `laplace sampler check`, so the two can't disagree.
- Hover, completion, go-to-definition and signature help. The names and
  docs come from `builtins()`, `combinators()` and `kernel_index()` in the
  same crate.
- Hover on `@adapt` explains warmup adaptation: when it runs, what freezes,
  and both tags. Hover on `step_size`, `metric` or `target_accept` inside
  `@adapt(...)` shows that tag's algorithm and schedule. Hover on `propose`
  and `move` explains the accept rule, the correction term, rejection, and
  how each interacts with `@adapt`.
- Hover on `q`, `lp`, `grad`, `dim`, `metric`, `log_density`,
  `log_density_grad` and `leapfrog` explains each one in full: its scale and
  convention, its cost, and its behaviour under a block.
- Kernels get laplace's new math library (`inv_logit`, `log1p_exp`,
  `log_sum_exp`, `fmax`, `softmax`, ... with hover docs). A kernel function
  with a library function's name shows as the kernel's own function.
- Parameter names inside `block(...)` come from the model's
  `parameters { }` block. The model is a `// model:` comment, else the
  `.laplace` file with the sampler's name, else the only `.laplace` file
  in its directory.

### Changed

- Depends on the `laplace` repo (the `laplace` and `laplace-sampler-syntax`
  crates) by git, pinned to one commit. See `docs/editor/merge-plan.md`
  for moving the pin to the `v0.3.0` tag.
- Completion now also triggers on `(`, `,` and `/`. In `.laplace` files a
  completion triggered by those characters returns nothing, so they behave
  as before.
- `.laplace` and `.laplacelib` behaviour is otherwise unchanged.

## 0.1.0

The `.laplace` / `.laplacelib` server: semantic tokens, completion,
hover, go-to-definition and diagnostics.
