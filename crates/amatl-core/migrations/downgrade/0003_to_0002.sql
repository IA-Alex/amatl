-- Downgrade from version 3 to version 2.
-- Removes domain persistence tables (search_history, saved_documents).
DROP TABLE IF EXISTS search_history;
DROP TABLE IF EXISTS saved_documents;
PRAGMA user_version = 2;
