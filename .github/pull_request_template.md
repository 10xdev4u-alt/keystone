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

## Visual QA (design-review gate)

Any change that touches UI ships with a pass of this checklist against the live preview:

- [ ] Renders correctly at 320px, 768px, and 1280px — no horizontal scroll, no clipped content.
- [ ] Touch targets ≥ 44px on `pointer: coarse` (buttons, nav links, inputs, revoke actions).
- [ ] Sticky nav stays usable; scrollable nav rows don't fight the page scroll.
- [ ] Dark theme passes (system toggle) where the screen has surfaces.
- [ ] Empty / error / loading states not regressed by the layout change.

## Testing

<!-- What was run locally: cargo test, specific test names, manual curl checks. -->
