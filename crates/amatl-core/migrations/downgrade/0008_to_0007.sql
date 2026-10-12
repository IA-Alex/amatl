DROP INDEX IF EXISTS provider_search_cache_created;
DROP INDEX IF EXISTS document_cache_created;
DELETE FROM amatl_schema_migrations WHERE version = 8;
PRAGMA user_version = 7;
