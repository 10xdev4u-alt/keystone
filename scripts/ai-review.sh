#!/usr/bin/env bash
# Keystone AI PR review — OpenAI-compatible endpoint, no API key required.
#
# Posts TWO comments per review:
#   1. A structured review (verdict + severity-tagged findings + suggestions).
#   2. A companion "second opinion" — honest views from a different angle,
#      pinging the PR author for a response.
#
# Advisory only — never blocks a merge.
#
# Usage:
#   scripts/ai-review.sh [base_ref]        # review HEAD vs base_ref (default origin/main)
#
# Env (all optional):
#   AI_REVIEW_ENDPOINT   default https://qwen2api-n.smanx.xx.kg/v1/chat/completions
#   AI_REVIEW_MODEL      default qwen3.8-max
#   AI_REVIEW_MAX_BYTES  default 60000 (diff truncation)
#   AI_REVIEW_PR         PR number to comment on (required in CI; the checkout
#                        is detached HEAD so gh cannot infer the PR itself)
#
# NOTE: the diff is sent to an external endpoint. Only run this on code you are
# willing to share with that service.
set -euo pipefail

BASE_REF="${1:-origin/main}"
ENDPOINT="${AI_REVIEW_ENDPOINT:-https://qwen2api-n.smanx.xx.kg/v1/chat/completions}"
MODEL="${AI_REVIEW_MODEL:-qwen3.8-max}"
MAX_BYTES="${AI_REVIEW_MAX_BYTES:-60000}"

DIFF="$(git diff "${BASE_REF}...HEAD" -- . ':(exclude)Cargo.lock' 2>/dev/null || true)"
if [ -z "${DIFF}" ]; then
  echo "no diff to review against ${BASE_REF}; skipping"
  exit 0
fi
if [ "${#DIFF}" -gt "${MAX_BYTES}" ]; then
  DIFF="${DIFF:0:${MAX_BYTES}}"
  echo "::warning::diff truncated to ${MAX_BYTES} bytes"
fi

# Single model call; returns the assistant's text. Handles both the plain
# OpenAI shape and the SSE-chunked stream this endpoint emits by default.
#
# The endpoint intermittently closes the connection mid-stream (curl exit 18
# "partial file"), sometimes after delivering the whole body. We retry with
# --retry-all-errors and keep whatever the last attempt delivered — jq can
# still parse a complete payload even when curl reports a partial transfer.
call_model() {
  local system="$1"
  local user="$2"
  local payload raw out

  payload="$(jq -n \
    --arg model "${MODEL}" \
    --arg system "${system}" \
    --arg user "${user}" \
    '{model: $model, messages: [{role: "system", content: $system}, {role: "user", content: $user}], temperature: 0.2, max_tokens: 2000}')"

  raw=""
  for attempt in 1 2 3; do
    raw="$(curl -sS -m 240 --retry 2 --retry-delay 2 --retry-all-errors \
      -H 'Content-Type: application/json' -d "${payload}" "${ENDPOINT}" 2>/dev/null || true)"
    if [ -n "${raw}" ]; then
      break
    fi
    echo "::warning::model call attempt ${attempt} returned nothing; retrying" >&2
    sleep 3
  done

  # 2>/dev/null on printf silences "write error: Broken pipe" noise from
  # pipefail when the downstream reader finishes early.
  out="$(printf '%s' "${raw}" 2>/dev/null | jq -r '.choices[0].message.content // empty' 2>/dev/null || true)"
  if [ -z "${out}" ]; then
    out="$(printf '%s' "${raw}" 2>/dev/null \
      | grep '^data: ' | sed 's/^data: //' | grep -v '^\[DONE\]$' \
      | jq -r '.choices[0].delta.content // empty' 2>/dev/null | tr -d '\n' || true)"
  fi
  if [ -z "${out}" ]; then
    out="$(printf '%s' "${raw}" 2>/dev/null | jq -r '.error.message // empty' 2>/dev/null || true)"
  fi
  if [ -z "${out}" ]; then
    out="review failed: unparseable response from ${ENDPOINT}"
    echo "::warning::${out}" >&2
  fi
  printf '%s\n' "${out}"
}

post_comment() {
  local body="$1"
  local gh_out
  # CI checks out in detached HEAD, so gh cannot infer the PR from the branch
  # — the workflow passes the PR number explicitly via AI_REVIEW_PR. That
  # value must be numeric to guard against argument injection.
  if command -v gh >/dev/null 2>&1 \
    && [[ "${GITHUB_REPOSITORY:-}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] \
    && [[ "${AI_REVIEW_PR:-}" =~ ^[0-9]+$ ]]; then
    gh_out="$(gh pr comment "${AI_REVIEW_PR}" --repo "${GITHUB_REPOSITORY}" --body "${body}" 2>&1)" || {
      # Never fail silently: a review tool that can't post must say so.
      echo "::error::failed to post PR comment: ${gh_out}" >&2
      printf '%s\n' "${body}" >&2
      return 1
    }
    return 0
  fi
  printf '%s\n' "${body}"
}

AUTHOR="$(command -v gh >/dev/null 2>&1 \
  && [ -n "${GITHUB_REPOSITORY:-}" ] \
  && [ -n "${AI_REVIEW_PR:-}" ] \
  && gh pr view "${AI_REVIEW_PR}" --repo "${GITHUB_REPOSITORY}" --json author -q '.author.login' 2>/dev/null || true)"
AUTHOR="${AUTHOR:-${GITHUB_ACTOR:-the author}}"

# ── Pass 1: structured review ────────────────────────────────────────────────
STRUCTURED="$(call_model \
  "You are a terse, opinionated senior Rust engineer doing PR review. You never invent findings; when unsure, you say so. Findings must reference real file:line from the diff. If a section has no findings, write 'None.'" \
  "Review this pull-request diff. Output markdown with EXACTLY these sections:

## Verdict
One sentence: approve / request changes / needs discussion, with the single biggest reason.

## Security
Severity-tagged findings (CRITICAL/HIGH/MEDIUM/LOW) — SQL injection, authz/IDOR, missing ownership checks, secrets or tokens in logs/commits, panics/unwrap on attacker-controlled input, CSRF, rate-limit gaps.

## Correctness
Logic bugs, wrong argument order, dead code, ignored errors, off-by-one, unhandled edge cases.

## Concurrency
Races, non-transactional multi-write, inconsistent counters.

## Rust idiom
Anything clippy -D warnings would reject; misuse of unsafe.

## Suggestions
3-5 concrete, actionable suggestions in priority order. Each: '**<short title>** — <what to change and why>'.

If the diff is entirely clean, reply with exactly: No issues found.

<diff>
${DIFF}
</diff>")"

STRUCTURED_BODY="## 🔍 AI Review — ${MODEL}

${STRUCTURED}

---
_Structured pass vs \`${BASE_REF}\` by \`scripts/ai-review.sh\`. Advisory only — non-blocking._"

if [ -z "${STRUCTURED}" ] || [[ "${STRUCTURED}" == review\ failed:* ]]; then
  echo "::warning::structured review unavailable (${STRUCTURED}) — skipping comment" >&2
else
  post_comment "${STRUCTURED_BODY}" && echo "posted structured review comment" || echo "structured comment fallback printed"
fi

# ── Pass 2: companion second opinion ─────────────────────────────────────────
COMPANION="$(call_model \
  "You are an independent senior engineer giving a SECOND opinion on a pull request. You have not seen the first reviewer's notes. Be honest and direct — your value is disagreeing constructively and catching what a checklist review misses. Never invent findings." \
  "Read this diff as a peer, not a checklist. Give an honest second opinion:

1. **What I like** — 2-3 things genuinely done well.
2. **What worries me** — design-level risks: future maintenance traps, coupling, performance cliff, upgrade pain, security posture beyond line-level bugs.
3. **What's missing** — tests, error handling, observability, docs, edge cases the author likely forgot.
4. **What I'd do differently** — the ONE change you'd insist on before merge, stated plainly.
5. **Biggest risk to this feature** — one sentence.

Keep it under 250 words. Plain markdown, headers for each numbered part.

<diff>
${DIFF}
</diff>")"

COMPANION_BODY="## 🧠 Companion Second Opinion

Hey @${AUTHOR} — here's an honest, independent take on this PR:

${COMPANION}

---
**Want my deeper take on any of these?** Reply here and I'll dig in — suggest a specific spot and I'll review that path with fresh eyes.

_Second pass vs \`${BASE_REF}\` by \`scripts/ai-review.sh\`. Advisory only — non-blocking._"

if [ -z "${COMPANION}" ] || [[ "${COMPANION}" == review\ failed:* ]]; then
  echo "::warning::companion review unavailable (${COMPANION}) — skipping comment" >&2
else
  post_comment "${COMPANION_BODY}" && echo "posted companion comment" || echo "companion comment fallback printed"
fi
