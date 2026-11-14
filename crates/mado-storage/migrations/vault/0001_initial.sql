-- Generated from mado-docs/spec/11-passwords-autofill.md section 2.2. Edit the spec first, then this file.
CREATE TABLE vault_meta (k TEXT PRIMARY KEY, v BLOB NOT NULL);       -- version, recovery_wrapped_vk, recovery_salt, created_at
CREATE TABLE items (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN ('login','passkey','card','address','identity','note','totp')),
  nonce BLOB NOT NULL, data BLOB NOT NULL,       -- AES-256-GCM(EK, nonce, json, aad = id || kind)
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, last_used_at INTEGER, deleted_at INTEGER
);
CREATE TABLE item_index (item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE, tag BLOB NOT NULL, PRIMARY KEY (item_id, tag));
-- tag = HMAC-SHA256(IK, "site:" || registrable_domain) or (IK, "rp:" || rpId); lets a locked-to-unlocked lookup find items without plaintext site names on disk
CREATE TABLE never_save (tag BLOB PRIMARY KEY);   -- HMAC of origins the user said Never on
