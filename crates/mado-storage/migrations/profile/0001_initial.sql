-- Generated from mado-docs/spec/03-storage.md section 3. Edit the spec first, then this file.
-- Sessions (04)
CREATE TABLE windows (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL,
  x INTEGER, y INTEGER, width INTEGER, height INTEGER, screen TEXT,
  layout TEXT NOT NULL, sidebar_width INTEGER, sidebar_mode TEXT NOT NULL,
  active_tab_id TEXT, sort INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE tabs (
  id TEXT PRIMARY KEY, window_id TEXT NOT NULL REFERENCES windows(id) ON DELETE CASCADE,
  group_id TEXT REFERENCES groups(id) ON DELETE SET NULL,
  pinned INTEGER NOT NULL DEFAULT 0, sort INTEGER NOT NULL,
  url TEXT NOT NULL, title TEXT NOT NULL, custom_title TEXT, favicon_url TEXT,
  pinned_home_url TEXT, opener_id TEXT, zoom REAL NOT NULL DEFAULT 1.0, muted INTEGER NOT NULL DEFAULT 0,
  session_blob BLOB, scroll_x INTEGER, scroll_y INTEGER, snapshot BLOB,
  last_active_at INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE groups (
  id TEXT PRIMARY KEY, window_id TEXT REFERENCES windows(id) ON DELETE SET NULL,
  title TEXT NOT NULL, color TEXT NOT NULL, collapsed INTEGER NOT NULL DEFAULT 0,
  saved INTEGER NOT NULL DEFAULT 1, open INTEGER NOT NULL DEFAULT 1, agent_task_id TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE saved_group_tabs (           -- tabs of a closed saved group
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE, sort INTEGER NOT NULL,
  url TEXT NOT NULL, title TEXT NOT NULL, favicon_url TEXT, PRIMARY KEY (group_id, sort)
);
CREATE TABLE closed_stack (                -- ⌘⇧T, newest first by id
  id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL CHECK (kind IN ('tab','group','window','session')),
  payload TEXT NOT NULL,                    -- JSON snapshot needed to restore
  closed_at INTEGER NOT NULL
);
CREATE TABLE session_journal (             -- crash-safe log of session changes
  seq INTEGER PRIMARY KEY AUTOINCREMENT, op TEXT NOT NULL, at INTEGER NOT NULL
);

-- History (12)
CREATE TABLE urls (
  id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, host TEXT NOT NULL, title TEXT,
  visit_count INTEGER NOT NULL DEFAULT 0, typed_count INTEGER NOT NULL DEFAULT 0,
  last_visit_at INTEGER, frecency REAL NOT NULL DEFAULT 0, hidden INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX urls_host ON urls(host);
CREATE INDEX urls_frecency ON urls(frecency DESC);
CREATE TABLE visits (
  id INTEGER PRIMARY KEY, url_id INTEGER NOT NULL REFERENCES urls(id) ON DELETE CASCADE,
  visited_at INTEGER NOT NULL, transition TEXT NOT NULL, from_visit_id INTEGER, tab_id TEXT,
  duration_ms INTEGER
);
CREATE INDEX visits_time ON visits(visited_at DESC);
CREATE VIRTUAL TABLE page_text USING fts5(
  title, url, body, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2'
);                                          -- rowid = urls.id
CREATE TABLE page_text_meta (url_id INTEGER PRIMARY KEY, bytes INTEGER NOT NULL, indexed_at INTEGER NOT NULL);
CREATE TABLE history_exclusions (host TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
CREATE TABLE search_terms (url_id INTEGER PRIMARY KEY, term TEXT NOT NULL, engine_id TEXT NOT NULL);
CREATE TABLE form_history (                 -- ordinary field autocomplete
  name TEXT NOT NULL, value TEXT NOT NULL, host TEXT NOT NULL, uses INTEGER NOT NULL, last_used_at INTEGER NOT NULL,
  PRIMARY KEY (name, value, host)
);

-- Bookmarks (12)
CREATE TABLE bookmarks (
  id TEXT PRIMARY KEY, parent_id TEXT REFERENCES bookmarks(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('folder','link','separator')), title TEXT NOT NULL, url TEXT,
  sort INTEGER NOT NULL, root TEXT CHECK (root IN ('bar','other')),
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX bookmarks_parent ON bookmarks(parent_id, sort);
CREATE INDEX bookmarks_url ON bookmarks(url);

-- Downloads (12)
CREATE TABLE downloads (
  id TEXT PRIMARY KEY, url TEXT NOT NULL, final_url TEXT, referrer TEXT, tab_url TEXT,
  path TEXT, mime TEXT, total_bytes INTEGER, received_bytes INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL CHECK (state IN ('in_progress','paused','complete','cancelled','failed','dangerous','blocked')),
  danger TEXT, error TEXT, resume_data BLOB, started_at INTEGER NOT NULL, ended_at INTEGER, removed INTEGER NOT NULL DEFAULT 0
);

-- Site settings and permissions (07)
CREATE TABLE site_settings (
  origin TEXT NOT NULL,                     -- scheme://host[:port] or a pattern ([*.]host)
  key TEXT NOT NULL,                        -- permission or content setting key
  value TEXT NOT NULL,                      -- 'allow' | 'block' | 'ask' | JSON for structured values
  embedder TEXT NOT NULL DEFAULT '',        -- top-level origin for embedded requests, '' = any
  last_used_at INTEGER, granted_at INTEGER NOT NULL, expires_at INTEGER,
  source TEXT NOT NULL CHECK (source IN ('user','default','extension','auto_revoked')),
  PRIMARY KEY (origin, key, embedder)
);
CREATE TABLE app_link_rules (origin TEXT NOT NULL, scheme TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (origin, scheme));
CREATE TABLE protocol_handlers (scheme TEXT NOT NULL, origin TEXT NOT NULL, template TEXT NOT NULL, is_default INTEGER NOT NULL, PRIMARY KEY (scheme, origin));
CREATE TABLE site_engine_rules (host TEXT PRIMARY KEY, target TEXT NOT NULL CHECK (target IN ('system_browser')), reason TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE zoom_levels (host TEXT PRIMARY KEY, zoom REAL NOT NULL);
CREATE TABLE reader_sites (host TEXT PRIMARY KEY, always INTEGER NOT NULL);

-- Settings (06)
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, schema_version INTEGER NOT NULL, updated_at INTEGER NOT NULL, controlled_by TEXT);
CREATE TABLE settings_log (id INTEGER PRIMARY KEY AUTOINCREMENT, key TEXT NOT NULL, old TEXT, new TEXT, origin TEXT NOT NULL, at INTEGER NOT NULL);

-- Extensions (09)
CREATE TABLE extensions (
  id TEXT PRIMARY KEY,                      -- 32 a-p letters
  version TEXT NOT NULL, name TEXT NOT NULL, source TEXT NOT NULL CHECK (source IN ('store','unpacked','crx_file')),
  enabled INTEGER NOT NULL, disabled_reason TEXT, private_allowed INTEGER NOT NULL DEFAULT 0,
  site_access TEXT NOT NULL,                -- 'on_click' | 'specific' | 'all'
  granted_permissions TEXT NOT NULL,        -- JSON
  granted_hosts TEXT NOT NULL,              -- JSON
  pinned INTEGER NOT NULL DEFAULT 0, toolbar_sort INTEGER,
  installed_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, update_url TEXT
);
CREATE TABLE extension_shortcuts (extension_id TEXT NOT NULL, command TEXT NOT NULL, keys TEXT, global INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (extension_id, command));
CREATE TABLE overlaps (category TEXT PRIMARY KEY, choice TEXT NOT NULL, extension_id TEXT, decided_at INTEGER NOT NULL);

-- Blocker (10)
CREATE TABLE blocker_site (host TEXT PRIMARY KEY, enabled INTEGER NOT NULL);
CREATE TABLE blocker_hidden (id INTEGER PRIMARY KEY, host TEXT NOT NULL, selector TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE blocker_custom_filters (id INTEGER PRIMARY KEY, line TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE blocker_stats (day INTEGER NOT NULL, host TEXT NOT NULL, ads INTEGER NOT NULL, trackers INTEGER NOT NULL, PRIMARY KEY (day, host));

-- Search engines (05)
CREATE TABLE search_engines (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, keyword TEXT, search_url TEXT NOT NULL, suggest_url TEXT,
  favicon_url TEXT, active INTEGER NOT NULL, is_default INTEGER NOT NULL DEFAULT 0, source TEXT NOT NULL CHECK (source IN ('builtin','user','opensearch','extension')),
  created_at INTEGER NOT NULL, last_used_at INTEGER
);

-- Favicons
CREATE TABLE favicons (page_host TEXT NOT NULL, icon_url TEXT NOT NULL, data BLOB NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL, fetched_at INTEGER NOT NULL, PRIMARY KEY (page_host, size));

-- AI (13)
CREATE TABLE ai_connections (id TEXT PRIMARY KEY, kind TEXT NOT NULL, provider TEXT NOT NULL, label TEXT NOT NULL, priority INTEGER NOT NULL, config TEXT NOT NULL, secret_ref TEXT, status TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE ai_models (connection_id TEXT NOT NULL, model_id TEXT NOT NULL, info TEXT NOT NULL, show INTEGER NOT NULL DEFAULT 1, starred INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (connection_id, model_id));
CREATE TABLE ai_roles (role TEXT PRIMARY KEY, model TEXT, effort TEXT, fallback TEXT);
CREATE TABLE ai_chats (id TEXT PRIMARY KEY, title TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, expires_at INTEGER);
CREATE TABLE ai_messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL REFERENCES ai_chats(id) ON DELETE CASCADE, role TEXT NOT NULL, content TEXT NOT NULL, model TEXT, created_at INTEGER NOT NULL);
CREATE TABLE ai_tasks (id TEXT PRIMARY KEY, prompt TEXT NOT NULL, status TEXT NOT NULL, mode TEXT NOT NULL, runtime TEXT NOT NULL, model TEXT, started_at INTEGER NOT NULL, ended_at INTEGER, answer TEXT, cost_micros INTEGER NOT NULL DEFAULT 0, scope TEXT NOT NULL);
CREATE TABLE ai_audit (id INTEGER PRIMARY KEY, task_id TEXT NOT NULL REFERENCES ai_tasks(id) ON DELETE CASCADE, at INTEGER NOT NULL, kind TEXT NOT NULL, detail TEXT NOT NULL, tokens_in INTEGER, tokens_out INTEGER, cost_micros INTEGER);
CREATE TABLE ai_routines (id TEXT PRIMARY KEY, prompt TEXT NOT NULL, schedule TEXT NOT NULL, mode TEXT NOT NULL, enabled INTEGER NOT NULL, last_run_at INTEGER, next_run_at INTEGER, created_at INTEGER NOT NULL);
CREATE TABLE ai_spend (day INTEGER NOT NULL, connection_id TEXT NOT NULL, cost_micros INTEGER NOT NULL, PRIMARY KEY (day, connection_id));

-- Control clients (14)
CREATE TABLE control_clients (id TEXT PRIMARY KEY, name TEXT NOT NULL, token_hash BLOB NOT NULL, scopes TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER, revoked INTEGER NOT NULL DEFAULT 0);

-- Sync readiness
CREATE TABLE change_log (seq INTEGER PRIMARY KEY AUTOINCREMENT, tbl TEXT NOT NULL, row_id TEXT NOT NULL, op TEXT NOT NULL CHECK (op IN ('insert','update','delete')), at INTEGER NOT NULL);

-- Change log (03 section 6): every table that will sync later.
CREATE TRIGGER change_log_bookmarks_insert AFTER INSERT ON bookmarks BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('bookmarks', NEW.id, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_bookmarks_update AFTER UPDATE ON bookmarks BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('bookmarks', NEW.id, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_bookmarks_delete AFTER DELETE ON bookmarks BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('bookmarks', OLD.id, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_settings_insert AFTER INSERT ON settings BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('settings', NEW.key, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_settings_update AFTER UPDATE ON settings BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('settings', NEW.key, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_settings_delete AFTER DELETE ON settings BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('settings', OLD.key, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_search_engines_insert AFTER INSERT ON search_engines BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('search_engines', NEW.id, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_search_engines_update AFTER UPDATE ON search_engines BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('search_engines', NEW.id, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_search_engines_delete AFTER DELETE ON search_engines BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('search_engines', OLD.id, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_extensions_insert AFTER INSERT ON extensions BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('extensions', NEW.id, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_extensions_update AFTER UPDATE ON extensions BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('extensions', NEW.id, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_extensions_delete AFTER DELETE ON extensions BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('extensions', OLD.id, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_groups_insert AFTER INSERT ON groups BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('groups', NEW.id, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_groups_update AFTER UPDATE ON groups BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('groups', NEW.id, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_groups_delete AFTER DELETE ON groups BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('groups', OLD.id, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_site_settings_insert AFTER INSERT ON site_settings BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('site_settings', NEW.origin || '|' || NEW.key || '|' || NEW.embedder, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_site_settings_update AFTER UPDATE ON site_settings BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('site_settings', NEW.origin || '|' || NEW.key || '|' || NEW.embedder, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_site_settings_delete AFTER DELETE ON site_settings BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('site_settings', OLD.origin || '|' || OLD.key || '|' || OLD.embedder, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_ai_routines_insert AFTER INSERT ON ai_routines BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('ai_routines', NEW.id, 'insert', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_ai_routines_update AFTER UPDATE ON ai_routines BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('ai_routines', NEW.id, 'update', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
CREATE TRIGGER change_log_ai_routines_delete AFTER DELETE ON ai_routines BEGIN
  INSERT INTO change_log (tbl, row_id, op, at) VALUES ('ai_routines', OLD.id, 'delete', CAST(unixepoch('subsec') * 1000 AS INTEGER));
END;
