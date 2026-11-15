CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR);
CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, place_id INTEGER, visit_date INTEGER);
CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title LONGVARCHAR, guid TEXT);
INSERT INTO moz_places (id, url, title) VALUES (1, 'https://www.mozilla.org/', 'Mozilla'), (2, 'https://developer.mozilla.org/', 'MDN'), (3, 'place:sort=8', NULL);
INSERT INTO moz_historyvisits (place_id, visit_date) VALUES (1, 1793491200000000), (2, 1793577600000000), (2, 1780000000000000);
INSERT INTO moz_bookmarks (id, type, fk, parent, position, title, guid) VALUES
  (1, 2, NULL, 0, 0, '', 'root________'),
  (2, 2, NULL, 1, 0, 'menu', 'menu________'),
  (3, 2, NULL, 1, 1, 'toolbar', 'toolbar_____'),
  (5, 2, NULL, 1, 3, 'unfiled', 'unfiled_____'),
  (10, 1, 1, 3, 0, 'Mozilla', 'aaaaaaaaaaaa'),
  (11, 2, NULL, 3, 1, 'Docs', 'bbbbbbbbbbbb'),
  (12, 1, 2, 11, 0, 'MDN', 'cccccccccccc'),
  (13, 1, 3, 3, 2, 'Most visited', 'dddddddddddd'),
  (14, 1, 2, 5, 0, 'MDN unfiled', 'eeeeeeeeeeee');
