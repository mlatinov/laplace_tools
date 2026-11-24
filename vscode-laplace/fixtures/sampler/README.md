# Sampler fixtures

Grammar snapshot fixtures for `.lapkernel` and `.lapsampler`
(`npm run test:grammar`). The real files are copied here, so the tests don't
depend on sibling checkouts:

| Folder | Source | Commit |
| --- | --- | --- |
| `laplace-sample/` | `laplace` repo: `laplace-sample/kernels/*.lapkernel`, `laplace-sample/tests/fixtures/broken_mala.lapkernel`, `laplace-sample/tests/targets/*.lapsampler` | `b2c481f` |
| `laplace-langevin/` | `laplace-langevin` repo: `kernels/*.lapkernel` | `0650f56` |
| `coverage/` | Written for these tests: every token class in each grammar (`docs/editor/phase-0-report.md` §0.2) | — |

`coverage/*.lapkernel` pass `laplace sampler check`. `coverage/coverage.lapsampler`
parses, but on purpose it names a package and local files that don't exist
here, and uses `on(...)`, which is refused inside a kernel line.

After a grammar change, run `npm run test:grammar:update` and review the
`.snap` diff.
