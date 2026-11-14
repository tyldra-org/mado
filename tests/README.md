# Tests

End-to-end tests, fixtures and sample data:

- `pages/`: fixture pages for engine events, permissions, forms, the blocker and attack cases.
- `e2e/`: a TypeScript harness that launches Mado and drives it through its control endpoint.
- `import/`: anonymised profile folders from other browsers.

Unit tests live next to the code they test.

## End-to-end tests

`bun run test:e2e` builds the app with the `test-hooks` feature and runs `tests/e2e`. Each test starts its own Mado in a temporary data folder (`MADO_DATA_DIR`), `--hidden`, with any window it opens kept hidden and unfocused (`MADO_TEST_HEADLESS`), and talks to it through the control endpoint with a random test token (`MADO_TEST_TOKEN`). Without `MADO_E2E_BIN`, `bun test` skips them. CI runs them in `.github/workflows/build.yml`.

## Fixture server

`cargo run -p mado-fixture-server -- --ca-out /tmp/mado-test-ca.pem` serves `pages/` on every `*.mado.test` host through one proxy port, over HTTP and HTTPS with a test CA it makes at start. `expired.mado.test`, `wrong-name.mado.test` and `self-signed.mado.test` present bad certificates on purpose. Tabs reach it through the profile's proxy setting from M2.
