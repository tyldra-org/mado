CREATE TABLE history_items (id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, domain_expansion TEXT, visit_count INTEGER NOT NULL);
CREATE TABLE history_visits (id INTEGER PRIMARY KEY, history_item INTEGER NOT NULL, visit_time REAL NOT NULL, title TEXT);
INSERT INTO history_items (id, url, visit_count) VALUES (1, 'https://www.apple.com/', 2), (2, 'https://webkit.org/', 1);
INSERT INTO history_visits (history_item, visit_time, title) VALUES (1, 815184000, 'Apple'), (1, 815270400, 'Apple'), (2, 801692800, 'WebKit');
