-- Settings an overlap choice or an extension takeover changed, so they go
-- back when the extension is turned off or removed (spec 09 §8).
ALTER TABLE overlaps ADD COLUMN previous TEXT NOT NULL DEFAULT '{}';
CREATE TABLE extension_takeovers (
  extension_id TEXT NOT NULL,
  what TEXT NOT NULL,                       -- 'newTab' | 'search' | 'startup'
  kept INTEGER NOT NULL,
  previous TEXT NOT NULL DEFAULT '{}',      -- JSON: setting key -> value before
  decided_at INTEGER NOT NULL,
  PRIMARY KEY (extension_id, what)
);
