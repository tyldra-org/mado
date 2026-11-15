CREATE TABLE keywords (id INTEGER PRIMARY KEY, short_name VARCHAR, keyword VARCHAR, favicon_url VARCHAR, url VARCHAR);
INSERT INTO keywords (short_name, keyword, url) VALUES ('Google', 'google.com', 'https://www.google.com/search?q={searchTerms}'), ('Wikipedia', 'w', 'https://en.wikipedia.org/w/index.php?search={searchTerms}'), ('History', 'h', 'chrome://history/?q={searchTerms}');
