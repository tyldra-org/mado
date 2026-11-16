# Bundled data

Read at build time by `src/idn.rs` (`include_str!`). Update with the
upstream files, then run `cargo nextest run -p mado-navigation`.

| File                     | Source                                                                         | Licence                      |
| ------------------------ | ------------------------------------------------------------------------------ | ---------------------------- |
| `confusables.txt`        | Unicode TR39 `confusables.txt` 18.0.0, reduced to `source;prototype` hex pairs | Unicode-3.0, © Unicode, Inc. |
| `identifier-allowed.txt` | Unicode TR39 `IdentifierStatus.txt` 18.0.0, the `Allowed` ranges               | Unicode-3.0, © Unicode, Inc. |

The top domains list is a signed list, in `lists/top-domains.txt`.

The spoof checks in `src/idn.rs` port Chromium's `IDNSpoofChecker` and
`SkeletonGenerator` (BSD-3-Clause, © The Chromium Authors). Both are listed
in `scripts/credits.ts` for `mado://credits`.
