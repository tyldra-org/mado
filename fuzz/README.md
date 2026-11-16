# Fuzz targets

Inputs Mado reads from the network or from other apps, fed random bytes by libFuzzer (spec 16 §1). Each target must refuse bad input without panicking or hanging.

| Target | Reads |
| --- | --- |
| `crx_header` | CRX3 packages (`mado-extensions::verify`, spec 09 §3) |

Run one for ten minutes, as the spec's nightly job will:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run crx_header -- -max_total_time=600
```

A crash leaves its input in `fuzz/artifacts/<target>/`; turn it into a unit test in the crate before fixing it. Ordinary tests also feed each parser random input with proptest, so CI catches the simplest cases on every pull request.
