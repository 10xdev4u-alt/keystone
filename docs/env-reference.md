# Environment Reference

Source of truth: `.env.example` (templates only — real secrets are generated,
never committed). Every variable is read at boot by `keystone-config` and the
API fails fast on invalid values (bad ports, missing JWT key, unreachable DB).

## Runtime

| Variable | Default | Required | Notes |
| -------- | ------- | -------- | ----- |
| `PORT` | `4000` | — | API listen port |
| `HOST` | `0.0.0.0` | — | Bind address |
| `RUST_LOG` | `info,keystone=debug` | — | tracing filter |
| `NODE_ENV` | `development` | — | legacy compatibility, not read by Rust |

## Database

| Variable | Default | Required | Notes |
| -------- | ------- | -------- | ----- |
| `DATABASE_URL` | — | **yes** | `postgres://user:pass@host:5432/db` |
| `DATABASE_MAX_CONNECTIONS` | `10` | — | sqlx pool size |
| `PG_CONNECTION_TIMEOUT_MS` | `30000` | — | connect timeout |
| `TEST_DATABASE_URL` | — | for tests | integration suites self-skip when unset |

Migrations run automatically on boot (`sqlx::migrate!`). Versioned with
down-migrations; never hand-edit applied migrations.

## Auth & tokens

| Variable | Default | Required | Notes |
| -------- | ------- | -------- | ----- |
| `JWT_PRIVATE_KEY_B64` | — | one of B64/PATH | base64 secret ≥ 32 bytes (`openssl rand -base64 48`) |
| `JWT_PRIVATE_KEY_PATH` | — | one of B64/PATH | file containing the base64 secret |
| `JWT_ACCESS_EXPIRATION` | `900` | — | access token TTL (s) |
| `JWT_REFRESH_EXPIRATION` | `604800` | — | refresh token TTL (s) |
| `JWT_ISSUER` | `keystone` | — | `iss` claim |
| `JWT_AUDIENCE` | `keystone-api` | — | `aud` claim |
| `ARGON2_MEMORY` | `65536` | — | KiB cost |
| `ARGON2_ITERATIONS` | `3` | — | time cost |
| `ARGON2_PARALLELISM` | `4` | — | threads |

The API **refuses to boot without a JWT key**. Generate per environment; do
not reuse keys across environments.

## OAuth (Google)

OAuth is disabled until **both** `OAUTH_GOOGLE_CLIENT_ID` and
`OAUTH_GOOGLE_CLIENT_SECRET` are set. All endpoints are overridable so tests
can point at a local mock provider.

| Variable | Default | Notes |
| -------- | ------- | ----- |
| `OAUTH_GOOGLE_CLIENT_ID` | — | enables `/api/v1/auth/oauth/google/start` |
| `OAUTH_GOOGLE_CLIENT_SECRET` | — | |
| `OAUTH_GOOGLE_REDIRECT_URI` | `{API_URL}/api/v1/auth/oauth/google/callback` | must match the Google console |
| `OAUTH_GOOGLE_AUTH_URL` | Google accounts endpoint | |
| `OAUTH_GOOGLE_TOKEN_URL` | Google token endpoint | |
| `OAUTH_GOOGLE_USERINFO_URL` | Google OpenID userinfo | |
| `OAUTH_GOOGLE_SCOPES` | `openid email profile` | |
| `APP_POST_LOGIN_REDIRECT` | `{APP_URL}/` | where OAuth logins land |

## Mail (SMTP)

| Variable | Default | Notes |
| -------- | ------- | ----- |
| `SMTP_HOST` | — | empty → mail is dev-printed (verification/reset tokens in response body) |
| `SMTP_PORT` | `587` | |
| `SMTP_USER` / `SMTP_PASS` | — | |
| `SMTP_FROM` | `noreply@keystone.app` | |

Dev mode: with no SMTP host, `register` returns the verification token in
the JSON body so local flows work end-to-end without a mail server.

## Object storage

Two backends via `STORAGE_BACKEND`:

| Variable | Default | Notes |
| -------- | ------- | ----- |
| `STORAGE_BACKEND` | `memory` | `memory` (dev, lost on restart) or `s3` |
| `STORAGE_BUCKET` | `keystone` | bucket for the `s3` backend |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | — | required when `s3` |
| `AWS_REGION` | `us-east-1` | |
| `AWS_ENDPOINT_URL_S3` | — | MinIO local endpoint |

Legacy `S3_*` variables exist in `.env.example` for the earlier presigned
design; the storage crate reads the `AWS_*` set. Keep both consistent when
running MinIO locally.

## App / CORS

| Variable | Default | Notes |
| -------- | ------- | ----- |
| `APP_URL` | `http://localhost:5173` | public frontend URL |
| `API_URL` | `http://localhost:4000` | public API URL |
| `CORS_ORIGINS` | — | comma-separated extra allowed origins |

## Observability

| Variable | Notes |
| -------- | ----- |
| `SENTRY_DSN` | optional |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | optional OTLP tracing |

## Secrets hygiene

- Only `.env.example` is committed; real envs live in the deployment secret store.
- Secrets are never logged (CI grep gate fails on secret-shaped output).
- All credentials are **newly generated** — no legacy HrX secrets are reused
  (see `docs/threat-model.md` and the env-scan report).
