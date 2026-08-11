-- 0007_search_storage.down.sql — reverse of the Month 8 schema
DROP TABLE IF EXISTS upload_quotas;
DROP TABLE IF EXISTS file_records;

ALTER TABLE courses DROP COLUMN IF EXISTS search_doc;
ALTER TABLE communities DROP COLUMN IF EXISTS search_doc;
ALTER TABLE users DROP COLUMN IF EXISTS search_doc;
ALTER TABLE posts DROP COLUMN IF EXISTS search_doc;
