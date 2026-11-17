-- Calls made with full control (spec 14 §2.1): who, when and what.
CREATE TABLE control_audit (id INTEGER PRIMARY KEY, client_id TEXT NOT NULL, client_name TEXT NOT NULL, at INTEGER NOT NULL, method TEXT NOT NULL, detail TEXT NOT NULL);
CREATE INDEX control_audit_at ON control_audit (at);
