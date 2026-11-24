// The languages the language server serves. `.laplace` and `.laplacelib`
// get every feature; `.lapkernel` and `.lapsampler` get diagnostics, from
// the same checker as `laplace sampler check` (the server answers nothing
// else for them). package.json's activationEvents list the same ids
// (checked by src/test/kernelLanguages.test.ts).
export const SERVER_LANGUAGES = ["laplace", "laplacelib", "lapkernel", "lapsampler"] as const;
