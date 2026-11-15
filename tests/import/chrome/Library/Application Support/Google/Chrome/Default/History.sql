CREATE TABLE urls (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, visit_count INTEGER DEFAULT 0, typed_count INTEGER DEFAULT 0, last_visit_time INTEGER, hidden INTEGER DEFAULT 0);
CREATE TABLE visits (id INTEGER PRIMARY KEY, url INTEGER NOT NULL, visit_time INTEGER NOT NULL, from_visit INTEGER, transition INTEGER DEFAULT 0);
INSERT INTO urls (id, url, title) VALUES (1, 'https://www.rust-lang.org/', 'Rust'), (2, 'https://docs.example/guide', 'Guide'), (3, 'https://old.example/', 'Old');
INSERT INTO visits (url, visit_time) VALUES (1, 13437964800000000), (1, 13438051200000000), (2, 13438051200000000), (3, 13424473600000000);
