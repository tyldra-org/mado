-- Tracker domains blocked per day, for the privacy report's top tracker
-- domains (spec 08 §9). blocker_stats keeps the per-site counts.
CREATE TABLE blocker_tracker_stats (
  day INTEGER NOT NULL,                     -- Unix days
  domain TEXT NOT NULL,                     -- registrable domain
  count INTEGER NOT NULL,
  PRIMARY KEY (day, domain)
);
