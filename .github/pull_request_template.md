## What

<!-- One or two sentences: what does this PR change and why? -->

## Definition of Done

- [ ] Schema change? Real FKs + CHECK + unique constraints; down-migration present; dry-run green on a fresh DB.
- [ ] All SQL through `sqlx::query!` — no dynamic SQL strings.
- [ ] API change? OpenAPI regenerated; generated TS client committed; zero hand-written frontend types.
- [ ] Tests in this PR (unit and/or integration); `cargo fmt` and `cargo clippy -D warnings` clean.
- [ ] Security checklist: ownership/IDOR check on every `/{id}` route; CSRF on state-changing requests; audit event where required; rate-limit tier assigned.
- [ ] Observability: tracing span for the new path; an operator-readable log line; histogram metric where latency matters.

## Review notes

<!-- Anything the reviewer should look at closely: tricky invariants, concurrency, migration ordering, backward compatibility. -->

## Testing

<!-- What was run locally: cargo test, specific test names, manual curl checks. -->
