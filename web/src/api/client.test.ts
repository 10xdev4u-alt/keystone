import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ApiRequestError,
  getAccessToken,
  isAuthenticated,
  onAuthChange,
  setTokens,
} from "./client";

afterEach(() => {
  setTokens(null, null);
  vi.restoreAllMocks();
});

describe("token lifecycle", () => {
  it("starts unauthenticated", () => {
    expect(isAuthenticated()).toBe(false);
    expect(getAccessToken()).toBeNull();
  });

  it("stores tokens in memory and flips auth state", () => {
    setTokens("access-1", "csrf-1");
    expect(isAuthenticated()).toBe(true);
    expect(getAccessToken()).toBe("access-1");
  });

  it("notifies subscribers on change", () => {
    const seen: boolean[] = [];
    const off = onAuthChange((authed) => seen.push(authed));
    setTokens("access-2", null);
    setTokens(null, null);
    expect(seen).toEqual([true, false]);
    off();
    setTokens("access-3", null);
    expect(seen).toEqual([true, false]);
  });
});

describe("ApiRequestError", () => {
  it("carries the RFC 7807 fields", () => {
    const err = new ApiRequestError(
      429,
      "https://keystone.dev/problems/rate-limited",
      "Too Many Requests",
      "Slow down",
    );
    expect(err.status).toBe(429);
    expect(err.type).toContain("rate-limited");
    expect(err.detail).toBe("Slow down");
    expect(err.message).toBe("Slow down");
    expect(err).toBeInstanceOf(Error);
  });

  it("falls back to the title as the message", () => {
    const err = new ApiRequestError(500, "about:blank", "Internal Server Error");
    expect(err.message).toBe("Internal Server Error");
  });
});
