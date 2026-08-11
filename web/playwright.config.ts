import { defineConfig } from "@playwright/test";

// E2E smoke suite: exercises the real API + web stack end-to-end.
//
// Playwright manages both servers (unless one is already running locally).
// - API: cargo run, auto-migrates a fresh Postgres schema at boot.
// - Web: Vite dev server; its proxy forwards /api to the API port, so the
//   browser stays same-origin (cookies + CSRF work exactly like production).
//
// The web server is the source of truth for the API port — keep 4311 in sync
// with web/vite.config.ts.
const API_PORT = 4311;
const WEB_PORT = 5173;

// Local default DB matches the test database used by the integration suites
// and the CI postgres service container. CI overrides via E2E_DATABASE_URL.
const DB_URL =
  process.env.E2E_DATABASE_URL ??
  "postgres://keystone:keystone_test@localhost:5432/keystone_test";

// Dev/test-only signing secret (>= 32 bytes when decoded). Never used in
// production — production keys are generated fresh and injected at deploy.
const JWT_SECRET =
  process.env.E2E_JWT_SECRET ??
  Buffer.from("keystone-e2e-dev-secret-key-0123456789abcdef").toString("base64");

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  timeout: 90_000,
  expect: { timeout: 10_000 },
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${WEB_PORT}`,
    trace: "retain-on-failure",
  },
  webServer: [
    {
      command: [
        `PORT=${API_PORT}`,
        `DATABASE_URL=${DB_URL}`,
        `JWT_PRIVATE_KEY_B64=${JWT_SECRET}`,
        "STORAGE_BACKEND=memory",
        `CORS_ORIGINS=http://127.0.0.1:${WEB_PORT}`,
        "cargo run --bin keystone-api",
      ].join(" "),
      cwd: "..",
      url: `http://127.0.0.1:${API_PORT}/healthz`,
      reuseExistingServer: !process.env.CI,
      timeout: 120_000,
    },
    {
      // --host 127.0.0.1: Vite 8 binds ::1 by default; Playwright polls the
      // IPv4 loopback, so pin the address explicitly.
      command: `npm run dev -- --port ${WEB_PORT} --strictPort --host 127.0.0.1`,
      url: `http://127.0.0.1:${WEB_PORT}`,
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
    },
  ],
});
