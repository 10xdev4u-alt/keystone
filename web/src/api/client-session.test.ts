import { afterEach, describe, expect, it, vi } from "vitest";

// Stub the fetch + base URL BEFORE the client module evaluates, so the
// middleware's refresh path uses the mock (openapi-fetch resolves its fetch
// from the global at request time, but ordering must not be assumed).
const { fetchMock } = vi.hoisted(() => ({ fetchMock: vi.fn() }));

vi.stubEnv("VITE_API_URL", "http://test.local");
vi.stubGlobal("fetch", fetchMock);

const { client, getAccessToken, setTokens } = await import("./client");

afterEach(() => {
  setTokens(null, null);
  fetchMock.mockReset();
});

describe("session recovery", () => {
  it("refreshes via cookie and replays a 401 on /auth/me", async () => {
    // Simulate the post-reload state: in-memory tokens empty, only the
    // (non-httpOnly) CSRF cookie present — the client must echo it back.
    // Path=/ matches the backend's cookie contract (JS-readable from any route).
    document.cookie = "keystone_csrf=cookie-csrf; Path=/";

    fetchMock
      .mockResolvedValueOnce(new Response("unauthorized", { status: 401 })) // /auth/me
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ access_token: "fresh", csrf_token: "csrf-new" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      ) // /auth/refresh
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ user: { id: "u1", email: "a@b.dev" } }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      ); // replayed /auth/me

    const { data, error } = await client.GET("/api/v1/auth/me");
    expect(error).toBeUndefined();
    expect((data as { user: { id: string } }).user.id).toBe("u1");
    expect(fetchMock).toHaveBeenCalledTimes(3); // me(401) → refresh → me(200)
    const calls = fetchMock.mock.calls.map((c) => {
      const input = c[0] as Request | string;
      return typeof input === "string" ? input : input.url;
    });
    expect(calls).toEqual([
      "http://test.local/api/v1/auth/me",
      "http://test.local/api/v1/auth/refresh",
      "http://test.local/api/v1/auth/me",
    ]);
    // The refresh request must carry the CSRF cookie value echoed back.
    const refreshCall = fetchMock.mock.calls.find((c) => String(c[0]).endsWith("/auth/refresh"));
    expect(new Headers((refreshCall?.[1] as RequestInit)?.headers).get("X-CSRF-Token")).toBe(
      "cookie-csrf",
    );
    expect(getAccessToken()).toBe("fresh");
  });
});
