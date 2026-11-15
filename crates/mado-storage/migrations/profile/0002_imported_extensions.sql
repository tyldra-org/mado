-- Extensions found while importing from another browser (spec 12 §5.2),
-- offered for reinstall once Mado has extensions (M3).
CREATE TABLE imported_extensions (
  id TEXT PRIMARY KEY,                      -- the store's extension ID
  name TEXT NOT NULL,
  browser TEXT NOT NULL,                    -- 'chrome', 'edge', 'brave', 'arc', 'firefox'
  imported_at INTEGER NOT NULL
);
