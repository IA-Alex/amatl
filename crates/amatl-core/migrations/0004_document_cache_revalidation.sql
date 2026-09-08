-- Add conditional revalidation support to document_cache.
-- Enables ETag/Last-Modified based cache revalidation.

ALTER TABLE document_cache ADD COLUMN etag TEXT;
ALTER TABLE document_cache ADD COLUMN last_modified TEXT;

PRAGMA user_version = 4;
