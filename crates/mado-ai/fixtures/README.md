# Provider stream fixtures

Server-sent event streams in each provider's documented format (spec 13 §3),
replayed by `src/client_tests.rs` through the mock server. Each provider has
two: `<api>-tool.sse` asks for the `page_title` tool, `<api>-text.sse`
answers after the tool result. They follow the providers' published
examples; no account recorded them, so replace one with a real capture when a
provider's format changes.
