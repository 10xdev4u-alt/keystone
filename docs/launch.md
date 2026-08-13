# Launch

How to take keystone from a repo checkout to a running production instance.
Complements [`operations.md`](operations.md) (day-two) and
[`env-reference.md`](env-reference.md) (every variable). This page is the
from-zero-to-live path.

## Topology

```
                    ┌─────────────────────────────┐
  users ── TLS ──► │  reverse proxy (Caddy/nginx) │
                    └──────────────┬──────────────┘
                                   │ HTTP/1.1 + WebSocket
                    ┌──────────────▼──────────────┐
                    │        keystone-api         │
                    │  (cargo release binary)     │
                    └───────┬──────────────┬──────┘
                            │              │
                 ┌──────────▼───┐   ┌──────▼──────┐
                 │  PostgreSQL  │   │ S3 / MinIO  │
                 │   16+        │   │ (uploads)   │
                 └──────────────┘   └─────────────┘
```

- **One API process.** The server assumes a single writer (migrations run on
  boot, `LISTEN/NOTIFY` fans out realtime). Scale read replicas later if you
  need it; do not run two boot-time-migrating instances at once.
- **Frontend is a static SPA** built by Vite; serve `web/dist` from any static
  host (or the reverse proxy) with `/api/*` and `/api/v1/ws/*` proxied to the
  API. CORS is configured from `CORS_ORIGINS` — see below.
- **WebSocket** (`/api/v1/ws/chat/{id}`) needs proxy config that upgrades
  connections and doesn't buffer frames. SSE (`/api/v1/notifications/feed`)
  needs buffering disabled too.

## 0. Prerequisites

- PostgreSQL 16+ reachable from the API host.
- Rust stable toolchain to build (or build in CI and copy the artifact).
- An S3-compatible bucket (real S3 or MinIO) if uploads are enabled;
  `STORAGE_BACKEND=memory` works for a demo but is not durable.

## 1. Build the API

```bash
# Release build — takes a while the first time
cargo build --release -p keystone-api
# Binary: target/release/keystone-api
```

The binary applies migrations on boot, so no separate migration step — but in
production **prefer running migrations explicitly first** (see §3) so a bad
migration fails the deploy instead of wedging the first request.

## 2. Environment

Copy `.env.example` to `.env` and set **at minimum**:

| Variable | Required | Notes |
| -------- | -------- | ----- |
| `DATABASE_URL` | yes | `postgres://user:pass@host:5432/keystone` |
| `JWT_PRIVATE_KEY_B64` | yes | `openssl rand -base64 48` — boot is fail-fast without it |
| `CORS_ORIGINS` | yes | comma-separated origins of the frontend, e.g. `https://app.example.com` |
| `STORAGE_BACKEND` | if uploads | `s3` + the `S3_*` block |
| `OAUTH_GOOGLE_*` | if Google login | set `OAUTH_GOOGLE_REDIRECT_URI` to the public callback URL |
| `SENTRY_DSN` / `OTEL_EXPORTER_OTLP_ENDPOINT` | optional | error/trace shipping |

The API refuses to boot with a missing JWT key — that's deliberate.

## 3. First admin

There is no user-role promotion endpoint by design; bootstrap the first
`super_admin` directly in SQL after the first user registers:

```sql
-- After the first user has registered, promote them:
UPDATE users SET role = 'super_admin' WHERE email = 'you@example.com';
```

Role vocabulary is CHECK-constrained: `user`, `moderator`, `admin`,
`super_admin`. A `super_admin` can then use the admin console UI
(`/admin`) to manage the rest of the staff.

## 4. Run

Use a process supervisor (systemd unit included below); do **not** run under
`nohup` in production.

```ini
# /etc/systemd/system/keystone.service
[Unit]
Description=keystone API
After=network-online.target
Wants=network-online.target

[Service]
User=keystone
WorkingDirectory=/opt/keystone
EnvironmentFile=/opt/keystone/.env
ExecStart=/opt/keystone/keystone-api
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
systemctl enable --now keystone
```

Health check after boot:

```bash
curl -fsS localhost:4000/healthz        # process up
curl -fsS localhost:4000/readyz         # DB reachable
curl -fsS localhost:4000/api/v1/health  # app JSON (status, uptime)
```

## 5. Reverse proxy (TLS)

Caddy (automatic HTTPS, WebSocket + SSE friendly):

```
app.example.com {
    handle /api/* {
        reverse_proxy 127.0.0.1:4000
    }
    handle /api/v1/ws/* {
        reverse_proxy 127.0.0.1:4000
    }
    handle {
        root * /opt/keystone/web/dist
        try_files {path} /index.html
        file_server
    }
}
```

nginx:

```nginx
server {
    listen 443 ssl;
    server_name app.example.com;
    # ssl_certificate / ssl_certificate_key ...

    location /api/v1/ws/ {
        proxy_pass http://127.0.0.1:4000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 3600s;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:4000;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location / {
        root /opt/keystone/web/dist;
        try_files $uri /index.html;
    }
}
```

Notes:

- The API sets `Strict-Transport-Security`, `X-Content-Type-Options`, and
  `X-Frame-Options` headers itself; let the proxy add `Referrer-Policy` if you
  want an extra layer.
- If the proxy strips the port from `Host`, cookies are unaffected (they are
  not domain-scoped); `CORS_ORIGINS` must list the exact origin the browser
  sees (scheme + host, no trailing slash).

## 6. Backups

See [`operations.md`](operations.md) — `pg_dump -Fc` on a schedule, mirror the
object store **before** restoring the DB, and enable `wal_level=replica` +
`archive_command` for point-in-time recovery. Test a restore at least once
before you need it.

## 7. Verify the surface

After launch, walk the critical paths once:

- Register → verify email → login → session appears on the profile page.
- Create a post → it shows on the feed; upload a cover → object lands in S3.
- Two browsers: chat a message → it appears live on the other side (WebSocket).
- Notification prefs: mute `comment` → a comment does not notify you.
- Admin: promote a user → their nav gains the staff menu.
- `cargo audit` clean, `GET /api/v1/admin/status` returns sane numbers.
