# Keystone — Threat Model v1

> Status: living document. Reviewed at the end of every milestone and before
> launch. Each threat maps to OWASP Top 10 / STRIDE and records whether the
> mitigation is **in place** (landed in code) or **planned** (designed, not
> yet implemented).

## Scope

The Keystone platform: Rust/Axum API, PostgreSQL, SPA frontend, S3 storage,
SSE/WebSocket realtime, SMTP mail, LLM assistant. Version 1 scope is the
top-20 module surface (identity, content, social, learning, mentorship,
events, admin).

## Assets

| Asset | Sensitivity | Where it lives |
|---|---|---|
| User credentials & password hashes | Critical | `users.password_hash` (Argon2id) |
| Refresh tokens | Critical | `sessions.refresh_token_hash` (SHA-256) |
| PII (email, name, education, salary) | High | `users`, profile tables, `salary_benchmarks` |
| Content & authorship | Medium | `posts`, `comments`, `reviews` |
| Audit trail | High | `audit_logs` (append-only) |
| Object storage files | Medium | S3/MinIO bucket |
| JWT signing keys | Critical | Files / env, never in repo |
| SMTP / OAuth / LLM credentials | Critical | Env only |

## Trust boundaries

```
Browser (SPA) ──HTTPS──▶ API ──SQL──▶ PostgreSQL
     │                    │  └──────▶ S3 (presigned)
     │                    ├─────────▶ SMTP (outbound only)
     └─SSE/WS────────────▶│  └──────▶ LLM (outbound only)
```

- **Boundary A — Browser ↔ API:** the only public boundary. Everything crossing
  it must be validated (schemas), authorized (ownership + roles), and rate-limited.
- **Boundary B — API ↔ PostgreSQL:** the app is the sole writer; the schema
  itself enforces integrity (FKs, CHECKs, triggers) as a second line of defense.
- **Boundary C — API ↔ external services:** outbound only; credentials never
  echoed in logs or responses.

## Threat register

| ID | STRIDE / OWASP | Scenario | Mitigation | Status |
|---|---|---|---|---|
| T-01 | Spoofing / A07 (auth) | Stolen session via XSS reading a token | Access token in memory only; refresh in httpOnly SameSite=Strict cookie; CSRF double-submit | **In place** (cookie policy designed; token storage is Month 2) |
| T-02 | Spoofing / A07 | Refresh token reuse | Rotation on every refresh; reuse revokes the whole session family | Planned (Month 2) |
| T-03 | Tampering / A02 | Forged/modified content IDs (IDOR) | Ownership + role checks on every `/{id}` route; security checklist item in every PR | Planned (all routes) |
| T-04 | Tampering / A03 | SQL injection via dynamic SQL | `sqlx::query!` compile-time checked; dynamic SQL banned | **In place** (policy in DoD) |
| T-05 | Info disclosure / A09 | Secrets in logs (tokens, passwords, keys) | CI grep gate; error model never serializes internals; no token logging | **In place** |
| T-06 | Info disclosure | Audit trail tampered | `audit_logs` append-only trigger in DB | **In place** |
| T-07 | Repudiation | Admin acts without a trace | Mandatory audit event on auth/admin actions; impersonation audited | Planned (Month 2) |
| T-08 | DoS / A05 | Brute-force login / credential stuffing | Argon2id (cost tuned); per-route rate limits; account lockout on repeated failures | Partially in place (lockout table `failed_logins` exists; limits are Month 2) |
| T-09 | DoS | Upload abuse / storage exhaustion | Per-user quotas, size/type validation, rate limits on upload | Planned (Month 8) |
| T-10 | DoS | Feed/dump endpoint abuse | Keyset pagination with caps; index-backed queries | Planned |
| T-11 | Tampering / A04 | Unsafe file access via presigned URLs | Strict expiry + scope on presigned URLs; path traversal defenses | Planned (Month 8) |
| T-12 | XSS / A03 | Stored XSS via rich content | Sanitizer on render; CSP for the SPA; no `dangerouslySetInnerHTML` without review | Planned (Month 9) |
| T-13 | Spoofing | OAuth account takeover via redirect manipulation | Strict redirect-URI allowlist server-side; no tokens in redirect URLs or logs | Planned (Month 2) |
| T-14 | Tampering | Salary/credit data manipulated | Integer minor units + currency; append-only credit ledger; transactional invariants | Planned |
| T-15 | Supply chain | Compromised dependency | `cargo audit` in CI; `cargo deny` policy; lockfile committed | **In place** (audit); deny pending |
| T-16 | Repudiation | Email verification/password reset token theft | Tokens stored hashed; single-use; expiry; never logged | **In place** (hashed columns in 0001; usage in Month 2) |

## Identity & auth decisions (T-01..T-03, T-13)

- Passwords: Argon2id, tuned params, no plaintext anywhere.
- Access tokens: RS256 JWT, 15-min TTL, keys loaded once at boot.
- Refresh tokens: opaque 256-bit random, stored hashed, httpOnly cookie only.
- Sessions: rotation + reuse detection, revocable, list/revoke-all.
- CSRF: double-submit token on all state-changing requests.
- Impersonation: admin-only, always audited.

## Out of scope for v1

- Multi-tenancy, client-side encryption of content, hardware security modules
  for keys (env-based keys are sufficient for v1), formal penetration test
  (replaced by the Month 12 security audit pass).

## Review cadence

- Every milestone: walk the register, update statuses, add threats.
- Every PR: the security checklist in the PR template.
- Pre-launch: full ASVS-style pass against this register (Month 12).
