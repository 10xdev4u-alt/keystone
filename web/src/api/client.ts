//! Typed API client generated from the backend OpenAPI spec.
//!
//! Zero hand-written API types: `generated.ts` is regenerated from
//! `/openapi.json` (see `scripts/generate-client.sh`) and `openapi-fetch`
//! wires the types to `fetch`.
//!
//! Security contract (mirrors the backend threat model):
//! - Access token lives in memory only — never localStorage/sessionStorage.
//! - Refresh happens via the httpOnly SameSite=Strict cookie (never read by
//!   JS); on 401 the middleware refreshes once and retries the request.
//! - CSRF double-submit token is sent on cookie-authenticated writes.

import createClient, { type Middleware } from "openapi-fetch";
import type { paths } from "./generated";

const BASE_URL: string =
  (import.meta.env.VITE_API_URL as string | undefined) ?? "";

/** In-memory access token + CSRF token. Cleared on logout or refresh failure. */
let accessToken: string | null = null;
let csrfToken: string | null = null;

/**
 * The `keystone_csrf` cookie is deliberately NOT httpOnly — the backend's
 * double-submit contract requires the SPA to echo it back. After a page
 * reload the in-memory token is gone, so reads fall back to the cookie.
 */
function readCsrfCookie(): string | null {
  if (typeof document === "undefined") return null;
  const match = document.cookie.match(/(?:^|; )keystone_csrf=([^;]+)/);
  return match ? decodeURIComponent(match[1]) : null;
}

function effectiveCsrf(): string | null {
  return csrfToken ?? readCsrfCookie();
}

const authListeners = new Set<(authed: boolean) => void>();

export function setTokens(access: string | null, csrf: string | null): void {
  accessToken = access;
  csrfToken = csrf;
  authListeners.forEach((l) => l(Boolean(access)));
}

export function getAccessToken(): string | null {
  return accessToken;
}

export function isAuthenticated(): boolean {
  return accessToken !== null;
}

/** Subscribe to auth state changes (for the shell to re-render on login/logout). */
export function onAuthChange(listener: (authed: boolean) => void): () => void {
  authListeners.add(listener);
  return () => authListeners.delete(listener);
}

/** RFC 7807 problem+json error surfaced to callers. */
export class ApiRequestError extends Error {
  readonly status: number;
  readonly type: string;
  readonly detail?: string;
  readonly headers: Headers;

  constructor(status: number, type: string, title: string, detail?: string, headers?: Headers) {
    super(detail ?? title);
    this.name = "ApiRequestError";
    this.status = status;
    this.type = type;
    this.detail = detail;
    this.headers = headers ?? new Headers();
  }
}

/** A single in-flight refresh guard so concurrent 401s trigger one call. */
let refreshing: Promise<boolean> | null = null;

async function refreshSession(): Promise<boolean> {
  if (refreshing) return refreshing;
  refreshing = (async () => {
    try {
      const csrf = effectiveCsrf();
      const res = await fetch(`${BASE_URL}/api/v1/auth/refresh`, {
        method: "POST",
        credentials: "include",
        headers: csrf ? { "X-CSRF-Token": csrf } : {},
      });
      if (!res.ok) {
        setTokens(null, null);
        return false;
      }
      const body = (await res.json()) as { access_token: string; csrf_token: string };
      setTokens(body.access_token, body.csrf_token);
      return true;
    } catch {
      setTokens(null, null);
      return false;
    } finally {
      refreshing = null;
    }
  })();
  return refreshing;
}

const authMiddleware: Middleware = {
  async onRequest({ request }) {
    if (accessToken) {
      request.headers.set("Authorization", `Bearer ${accessToken}`);
    }
    const csrf = effectiveCsrf();
    if (csrf && !["GET", "HEAD", "OPTIONS"].includes(request.method)) {
      request.headers.set("X-CSRF-Token", csrf);
    }
    return request;
  },
  async onResponse({ request, response }) {
    // Refresh exactly once on 401, then replay the original request. Only the
    // refresh endpoint itself is excluded — /auth/me 401s (e.g. after a page
    // reload wiped the in-memory token) must trigger the cookie refresh so a
    // valid session is recovered. The `refreshing` guard bounds the retry.
    if (response.status === 401 && !request.url.includes("/auth/refresh")) {
      if (await refreshSession()) {
        const retry = request.clone();
        if (accessToken) {
          retry.headers.set("Authorization", `Bearer ${accessToken}`);
        }
        const csrf = effectiveCsrf();
        if (csrf) {
          retry.headers.set("X-CSRF-Token", csrf);
        }
        return fetch(retry);
      }
    }
    if (!response.ok) {
      const problem = (await response
        .clone()
        .json()
        .catch(() => null)) as { type?: string; title?: string; detail?: string } | null;
      throw new ApiRequestError(
        response.status,
        problem?.type ?? "about:blank",
        problem?.title ?? response.statusText,
        problem?.detail,
        response.headers,
      );
    }
    return response;
  },
};

export const client = createClient<paths>({ baseUrl: BASE_URL, credentials: "include" });
client.use(authMiddleware);
