# Operations

Backup/restore, monitoring, and the day-two runbook for a keystone instance.

## Backup

The database is the source of truth for every user, post, session, and audit
event — back it up before any risky migration and on a schedule.

### Logical dump (pg_dump)

```bash
# Full dump, consistent snapshot
pg_dump -Fc -d "$DATABASE_URL" -f keystone-$(date +%Y%m%d-%H%M%S).dump

# Restore
pg_restore -d "$NEW_DATABASE_URL" --clean --if-exists keystone-YYYYMMDD-HHMMSS.dump
```

### Object storage

`STORAGE_BACKEND=s3` holds uploaded files. Back up the bucket separately and
restore **before** restoring the DB (post rows reference object keys — a DB
without its objects renders broken media, an object store without its DB
records leaks orphans).

MinIO/S3 backup:
```bash
# minio client (or aws s3 sync)
mc mirror --overwrite myminio/keystone-uploads /backup/keystone-uploads
```

### Point-in-time recovery

Enable Postgres `wal_level=replica` + `archive_command` in production so a
corrupt migration or operator error can be rolled back to minutes-before.
The migration history is checksummed — never edit an applied migration; add a
new one (reversible, with a down-migration).

## Monitoring

### Health endpoints

| Endpoint | Purpose |
| -------- | ------- |
| `GET /healthz` | liveness — process up |
| `GET /readyz` | readiness — DB reachable |
| `GET /api/v1/health` | app health JSON (status, uptime) |
| `GET /api/v1/admin/status` | typed ops view: status, uptime, user count, live sessions (super_admin) |

### Signals that matter

- **Auth failures** — `auth.login_failed` and `auth.change_password_failed`
  audit events spike = credential stuffing / account-targeted attacks. Rate
  limiter + lockout policy apply on the API; alert on the spike regardless.
- **Locked accounts** — `failed_logins` backoff is by user; a mass lockout
  wave suggests a distributed attack, not a password mistake.
- **Session revocations** — `auth.session_revoked` / `auth.sessions_revoked_all`
  are normal after password change, alarming when they happen for users who
  didn't act — stolen-session indicator.
- **Realtime lag** — SSE/WebSocket notifications ride `LISTEN/NOTIFY`; a
  backlog here surfaces as slow notification delivery, not API errors.
- **Disk** — uploads + audit_logs grow monotonically. Set capacity alerts on
  the volume hosting the DB and the object store.

### Tracing & errors

- `RUST_LOG` filter at runtime; structured fields carry actor/entity ids.
- `OTEL_EXPORTER_OTLP_ENDPOINT` exports traces to any OTLP collector.
- `SENTRY_DSN` enables error ingestion.

## Runbook

### "API won't boot: key error: no JWT key configured"

`JWT_PRIVATE_KEY_B64`/`PATH` unset. Generate: `openssl rand -base64 48`.
Boot is fail-fast by design — no half-configured production process.

### "Address already in use"

Port taken. Check `PORT` and the process table; the API never binds a second
instance (single writer assumption).

### "Migrations not applying"

`_sqlx_migrations` is checksummed. If a migration was edited after apply, the
boot fails loudly. Fix by adding a corrective migration, never by editing the
applied one.

### "Tests silently skip"

Integration suites self-skip when `TEST_DATABASE_URL` is unset/unreachable.
CI always sets it — a green-but-empty local run is a config gap, not a pass.

### "Sessions page shows devices I don't own"

That's the feature. Revoke the session (it dies immediately, family-wide),
then change your password — `change-password` revokes every other session
atomically.
