#!/usr/bin/env bash
# Regenerate the frontend API client from the backend OpenAPI spec.
#
# The spec is generated at compile time from the #[utoipa::path] annotations,
# so no database or running server is required:
#
#   cargo run -p keystone-api --bin keystone-dump-openapi > web/openapi.json
#   cd web && npx openapi-typescript ./openapi.json -o ./src/api/generated.ts
#
# CI runs this and fails on a diff: any backend route/schema change without a
# regenerated client breaks the build by design.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> dumping OpenAPI spec"
cargo run -q -p keystone-api --bin keystone-dump-openapi > web/openapi.json

echo "==> generating TS client"
(
  cd web
  npx openapi-typescript ./openapi.json -o ./src/api/generated.ts
)

echo "==> done (web/src/api/generated.ts regenerated)"
