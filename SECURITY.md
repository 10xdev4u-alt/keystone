# Security

## Reporting a vulnerability

Do **not** open a public issue for security problems. Report privately to the
repository owner (10xdev4u-alt) via GitHub security advisory or direct message.
Include: affected endpoint/route, repro steps, expected vs actual behavior, and
impact. A fix is expected within 7 days of confirmation.

## Posture

- **No unsafe code:** `#![forbid(unsafe_code)]` across the workspace; clippy
  `-D warnings` on every PR.
- **Database-enforced integrity:** real foreign keys, CHECK constraints on every
  enum column, unique constraints, append-only audit log (trigger-protected).
- **Auth:** Argon2id password hashing; RS256 JWTs with keys loaded once at boot;
  opaque refresh tokens stored hashed; httpOnly SameSite=Strict cookies; CSRF
  double-submit; per-route rate limiting; account lockout.
- **Secrets:** never committed (only `.env.example` templates), never logged (CI
  grep gate), rotated before first production use.
- **Dependencies:** `cargo audit` gate in CI; license/advisory policy enforced.

## Rules for contributors

1. Ownership checks on every `/{id}` route (IDOR) — no trust of client-supplied ids.
2. CSRF token required on all state-changing requests.
3. Audit events for auth-sensitive and admin actions.
4. Rate-limit tier assigned to every new route.
5. No tokens, passwords, or secrets in log lines, errors, or responses.
