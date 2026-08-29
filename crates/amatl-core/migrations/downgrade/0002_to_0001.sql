-- Downgrade from version 2 to version 1.
-- Removes the document_cache table added in phase 5.
DROP TABLE IF EXISTS document_cache;
PRAGMA user_version = 1;
