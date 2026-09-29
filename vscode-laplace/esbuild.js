// Bundles the extension into a single `out/extension.js`.
//
// The extension gained a runtime dependency on `mathjax-full`, which is 43MB
// installed -- shipping `node_modules` in the VSIX, as this extension used
// to, would have put all of it in the package. esbuild pulls in only the
// MathJax modules actually reached from `src/`, so the VSIX stays small and
// `vsce package --no-dependencies` is once again the right command: nothing
// is `require`d at runtime except `vscode` and Node's own built-ins.
//
// esbuild does not type-check. `npm run check` (tsc --noEmit) does, and
// `npm test` runs it.

const esbuild = require("esbuild");

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");
const tests = process.argv.includes("--tests");

// `mathjax-full/js/components/version.js` reads its own version by
// `eval('require')`-ing `../../package.json` -- deliberately hidden from
// bundlers, which are expected to supply it as a `PACKAGE_VERSION` global
// instead. Without this define, the eval'd require runs at load time,
// resolves `../../package.json` relative to `out/` rather than to
// `node_modules/mathjax-full/js/components/`, and throws MODULE_NOT_FOUND
// the first time a hover touches MathJax.
const MATHJAX_VERSION = require("mathjax-full/package.json").version;

/** @type {import("esbuild").BuildOptions} */
const common = {
  bundle: true,
  platform: "node",
  // The `engines.vscode` floor is 1.85, which ships Node 18.
  target: "node18",
  format: "cjs",
  // Provided by the editor at runtime, never bundled.
  external: ["vscode"],
  define: { PACKAGE_VERSION: JSON.stringify(MATHJAX_VERSION) },
  logLevel: "info",
};

/** @type {import("esbuild").BuildOptions} */
const extension = {
  ...common,
  entryPoints: ["src/extension.ts"],
  outfile: "out/extension.js",
  sourcemap: !production,
  minify: production,
};

/** The pure modules, built so `node --test` can run them without an editor. */
const unitTests = {
  ...common,
  entryPoints: [
    "src/test/math.test.ts",
    "src/test/docs.test.ts",
    "src/test/diagnosticFilter.test.ts",
    "src/test/languageConfig.test.ts",
  ],
  outdir: "out/test",
  sourcemap: true,
};

async function main() {
  const config = tests ? unitTests : extension;
  if (watch) {
    const ctx = await esbuild.context(config);
    await ctx.watch();
    return;
  }
  // A production build emits no sourcemap, so a map left over from an earlier
  // dev build would linger in `out/` -- and `vsce` would package it, which is
  // how a 5MB `extension.js.map` ended up in the VSIX once.
  if (!tests && production) {
    require("node:fs").rmSync("out/extension.js.map", { force: true });
  }
  await esbuild.build(config);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
