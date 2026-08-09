# keystone

An HR platform rebuilt from scratch. Rust API, PostgreSQL with real constraints, and a
brand-new frontend — engineered for correctness, security, and a UI that ships what the
architecture promises.

## Stack

| Layer      | Technology |
| ---------- | ---------- |
| API        | Rust (Axum, tokio) |
| Database   | PostgreSQL 16, sqlx (compile-time checked SQL), real FKs + CHECK constraints |
| Realtime   | SSE (notifications) + WebSocket (chat) over Postgres `LISTEN/NOTIFY` |
| Search     | Postgres full-text search (Elasticsearch pluggable behind a trait) |
| Storage    | S3/MinIO via aws-sdk-rust (presigned uploads/downloads) |
| Frontend   | React + Vite SPA, generated client from the OpenAPI spec (in progress) |
| CI/CD      | GitHub Actions: fmt, clippy `-D warnings`, tests, cargo audit |

## Repository layout

```
crates/api/       Axum app: routing, middleware, OpenAPI (utoipa)
crates/db/        Connection pool, migrations, repositories
crates/domain/    Pure domain types and business rules (no I/O)
migrations →      crates/db/migrations (versioned, checksummed, with down-migrations)
```

## Quick start

```bash
# 1. Start PostgreSQL (16+)
docker run -d --name keystone-pg -p 5432:5432 \
  -e POSTGRES_USER=keystone -e POSTGRES_PASSWORD=keystone -e POSTGRES_DB=keystone \
  postgres:16-alpine

# 2. Configure (templates only — real secrets are generated later, never committed)
cp .env.example .env        # then set DATABASE_URL

# 3. Run (applies migrations on boot)
cargo run -p keystone-api

# Health endpoints
curl localhost:4000/healthz        # liveness
curl localhost:4000/readyz         # readiness (checks DB)
curl localhost:4000/api/v1/health  # app health JSON
```

## Security posture

- Argon2id password hashing, RS256 JWTs (keys loaded once at boot), opaque refresh tokens
  stored hashed, httpOnly cookies, CSRF double-submit.
- `#![forbid(unsafe_code)]` across the workspace; clippy `-D warnings` on every PR.
- Every schema enum has a CHECK constraint; every relationship has a real foreign key.
- `audit_logs` is append-only, enforced by a database trigger.
- Secrets never logged (CI grep gate), real envs never committed (only `.env.example`).

## Contribution

- PR-driven only: branch → PR → CI green → review → merge. No direct pushes to `main`.
- Commits follow the conventional format with six-word subjects (e.g. `feat: add sqlx baseline migration`).
- Definition of Done lives in the project plan; every PR must keep CI green and pass the
  security checklist (IDOR checks, CSRF on writes, audit events, rate-limit tiers).
