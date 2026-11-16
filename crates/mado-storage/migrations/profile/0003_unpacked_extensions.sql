-- Unpacked extensions load from their own folder (spec 09 §2.3), so the row
-- keeps it. Store and .crx installs live under <profile>/Extensions/<id>/.
ALTER TABLE extensions ADD COLUMN path TEXT;
