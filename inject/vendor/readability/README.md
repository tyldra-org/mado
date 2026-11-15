# Readability

Mozilla Readability 0.6.0 (Apache-2.0, see LICENSE.md), copied unchanged from
the `@mozilla/readability` package: `Readability.js` and
`Readability-readerable.js`. The `reader` and `reader-probe` page scripts
bundle them (spec 08 §10). They live here, not in `node_modules`, so that
`cargo build` bundles the page scripts without `bun install`
(`crates/mado-engine/build.rs`). The `.d.ts` files are Mado's, taken from
the package's `index.d.ts`.

To update: copy both files and LICENSE.md from the new release, update the
version here and in `scripts/credits.ts`, and run the reader tests.
