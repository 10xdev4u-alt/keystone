#!/usr/bin/env bash
# Keystone AI code review — OpenAI-compatible endpoint, no API key required.
#
# Sends the current branch's diff (vs a base ref) to the configured endpoint and
# posts a concise security/correctness review as a pull-request comment.
# Advisory only — never blocks a merge.
#
# Usage:
#   scripts/ai-review.sh [base_ref]        # review HEAD vs base_ref (default origin/main)
#
# Env (all optional):
#   AI_REVIEW_ENDPOINT   default https://qwen2api-n.smanx.xx.kg/v1/chat/completions
#   AI_REVIEW_MODEL      default qwen3.7-max
#   AI_REVIEW_MAX_BYTES  default 60000 (diff truncation)
#
# NOTE: the diff is sent to an external endpoint. Only run this on code you are
# willing to share with that service.
set -euo pipefail

BASE_REF="${1:-origin/main}"
ENDPOINT="${AI_REVIEW_ENDPOINT:-https://qwen2api-n.smanx.xx.kg/v1/chat/completions}"
MODEL="${AI_REVIEW_MODEL:-qwen3.7-max}"
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

PAYLOAD="$(jq -n \
  --arg model "${MODEL}" \
  --arg system "You are a terse, opinionated senior Rust engineer doing PR review. You never invent findings; when unsure, you say so." \
  --arg user "Review this pull-request diff for, in priority order:
1. Security: SQL injection, authz/IDOR, missing ownership checks, secrets or tokens in logs/commits, panics/unwrap on attacker-controlled input, CSRF or rate-limit gaps.
2. Correctness: logic bugs, wrong argument order, dead code, ignored errors, off-by-one, unhandled edge cases.
3. Concurrency: races, non-transactional multi-write, inconsistent counters.
4. Rust idiom: anything clippy -D warnings would reject; misuse of unsafe.

Output format — no preamble, one line per finding:
- [SEV] file:line — finding
SEV in {CRITICAL, HIGH, MEDIUM, LOW}. If the diff is clean, reply with exactly: No issues found.

<diff>
${DIFF}
</diff>" \
  '{model: $model, messages: [{role: "system", content: $system}, {role: "user", content: $user}], temperature: 0.2, max_tokens: 2000}')"

RAW="$(curl -sS -m 240 -H 'Content-Type: application/json' -d "${PAYLOAD}" "${ENDPOINT}")"

# The endpoint answers in OpenAI streaming chunks even without stream:true.
# Prefer the non-stream shape, then concatenate SSE deltas.
REVIEW="$(printf '%s' "${RAW}" | jq -r '.choices[0].message.content // empty' 2>/dev/null || true)"
if [ -z "${REVIEW}" ]; then
  REVIEW="$(printf '%s' "${RAW}" \
    | grep '^data: ' | sed 's/^data: //' | grep -v '^\[DONE\]$' \
    | jq -r '.choices[0].delta.content // empty' 2>/dev/null | tr -d '\n' || true)"
fi
if [ -z "${REVIEW}" ]; then
  REVIEW="$(printf '%s' "${RAW}" | jq -r '.error.message // empty' 2>/dev/null || true)"
fi
if [ -z "${REVIEW}" ]; then
  REVIEW="review failed: unparseable response from ${ENDPOINT}"
  echo "::warning::${REVIEW}" >&2
fi

BODY="## 🤖 AI Review (${MODEL})

${REVIEW}

---
_Reviewed against \`${BASE_REF}\` by \`scripts/ai-review.sh\`. Advisory only — non-blocking._"

if command -v gh >/dev/null 2>&1 && [ -n "${GITHUB_REPOSITORY:-}" ]; then
  if gh pr comment --repo "${GITHUB_REPOSITORY}" --body "${BODY}" >/dev/null 2>&1; then
    echo "posted review comment"
  else
    printf '%s\n' "${BODY}"
  fi
else
  printf '%s\n' "${BODY}"
fi
