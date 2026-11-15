# Tests

End-to-end tests, fixtures and sample data:

- `pages/`: fixture pages for engine events, permissions, forms, the blocker and attack cases.
- `e2e/`: a TypeScript harness that launches Mado and drives it through its control endpoint.
- `import/`: anonymised profile folders from other browsers.

Unit tests live next to the code they test.

## End-to-end tests

`bun run test:e2e` builds the app with the `test-hooks` feature and runs `tests/e2e`. Each test starts its own Mado in a temporary data folder (`MADO_DATA_DIR`), `--hidden`, with any window it opens kept hidden and unfocused (`MADO_TEST_HEADLESS`), and talks to it through the control endpoint with a random test token (`MADO_TEST_TOKEN`). Without `MADO_E2E_BIN`, `bun test` skips them. Fixture pages use `*.mado.test` hosts only, so no test reaches a real site.

## Network audit

`bun run test:audit` runs the same suite with all of Mado's traffic, its own and its pages', sent through a recording fixture proxy (`MADO_AUDIT_PROXY`, `MADO_NETWORK_RECORD`), and fails on any host outside `network-allowlist.json` or the bundled search engines (spec 08 §11). The release workflow runs it before every release.

## Fixture server

`cargo run -p mado-fixture-server -- --ca-out /tmp/mado-test-ca.pem` serves `pages/` on every `*.mado.test` host through one proxy port, over HTTP and HTTPS with a test CA it makes at start. `expired.mado.test`, `wrong-name.mado.test` and `self-signed.mado.test` present bad certificates on purpose. Tabs reach it through the profile's proxy setting from M2.
