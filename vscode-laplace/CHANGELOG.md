# Changelog

## 0.5.0 (unreleased)

Needs `laplace-lsp` 0.2.0 for the sampler-file features. With an older
server, sampler files get highlighting and snippets only.

### Added: sampler files, experimental

Custom samplers are experimental in `laplace` 0.3.0, and so is this support.

- Two new languages: `.lapkernel` (a custom kernel) and `.lapsampler`
  (which kernel updates which parameters).
  - Highlighting.
  - Comments, brackets and indentation.
  - Snippets. `kernel` and `kernel-direct` are what
    `laplace new kernel` writes.
  - The Laplace file icon in the "Laplace (Seti)" icon theme, as for
    `.laplace` and `.laplacelib`.
- Live diagnostics, from the same code as `laplace sampler check`. A sampler
  names kernel files and packages; those are read and checked too. Saving a
  kernel re-checks the open samplers.
- Hover:
  - in `.lapkernel`, the built-ins;
  - in `.lapsampler`, kernels (what `laplace sampler doc` prints),
    combinators and settings.
- Completion:
  - in `.lapkernel`, built-ins (in `tuning { }`, only those a default may
    use);
  - in `.lapsampler`: `block`/`rest`; kernels and combinators; a kernel's
    settings with their defaults; and the model's parameters inside
    `block(...)`.
- Go to definition from a kernel in a `.lapsampler`:
  - a package or local kernel opens its `.lapkernel`;
  - a built-in opens its Rust source, read-only.
- Signature help for combinators and kernel calls.

### Changed

- The extension now also activates on the two new languages.
- The `.laplace` and `.laplacelib` support is unchanged.

## 0.4.0 and earlier

See the git history of `vscode-laplace/`.
