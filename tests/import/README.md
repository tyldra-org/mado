# Import fixtures

Fake home folders, one per browser, in the layout each browser writes on
macOS (spec 12 §5). The tests in `crates/mado-import` copy one to a
temporary folder and turn the text files into what the browser writes:

- `X.sql` becomes the SQLite database `X`.
- `X.snss.json` becomes the Chromium session file `X` (SNSS commands).
- `X.mozlz4.json` and `X.jsonlz4.json` become mozLz4 files.
- `X.fixture` is copied as `X`.

Everything here is made up; no file comes from a real profile.
