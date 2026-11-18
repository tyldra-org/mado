-- The memory search index (spec 13 §10): one row per paragraph of each
-- memory file, rebuilt for a file when its modification time changes.
CREATE VIRTUAL TABLE ai_memory_fts USING fts5(path UNINDEXED, heading, body, line UNINDEXED);
CREATE TABLE ai_memory_files (path TEXT PRIMARY KEY, mtime INTEGER NOT NULL);
