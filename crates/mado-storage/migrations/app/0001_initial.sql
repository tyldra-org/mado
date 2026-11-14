-- Generated from mado-docs/spec/12-profiles-and-data.md section 1.1. Edit the spec first, then this file.
CREATE TABLE profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL, avatar TEXT NOT NULL, sort INTEGER NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER, schema_version INTEGER NOT NULL);
CREATE TABLE link_rules (id TEXT PRIMARY KEY, pattern TEXT NOT NULL, profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE, sort INTEGER NOT NULL);
CREATE TABLE app_state (k TEXT PRIMARY KEY, v TEXT NOT NULL);   -- last_profile_id, clean_quit, install_id
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, schema_version INTEGER NOT NULL, updated_at INTEGER NOT NULL, controlled_by TEXT);
