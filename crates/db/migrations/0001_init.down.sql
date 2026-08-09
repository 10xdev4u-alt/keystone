-- 0001_init_down.sql — rollback for the identity & audit baseline.

DROP TRIGGER IF EXISTS users_set_updated_at ON users;
DROP FUNCTION IF EXISTS set_updated_at();

DROP TRIGGER IF EXISTS audit_logs_append_only_trigger ON audit_logs;
DROP FUNCTION IF EXISTS audit_logs_append_only();

DROP TABLE IF EXISTS failed_logins;
DROP TABLE IF EXISTS password_resets;
DROP TABLE IF EXISTS email_verifications;
DROP TABLE IF EXISTS audit_logs;
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS users;
