//! E2E smoke suite — the public journey, end-to-end through the real UI.
//!
//! register → verify → login → first post → comment → settings → sessions.
//! This is the plan's "e2e smoke suite green on the public journey" gate:
//! every step exercises the same surface a real user touches, backed by the
//! real API + Postgres (no mocks). The dev register flow returns the
//! verification token in the response body, which the UI's /verify token
//! input accepts — so even email verification stays in-browser.

import { expect, test } from "@playwright/test";

const PASSWORD = "E2e-Str0ng!pass-2026";

test.describe("public journey", () => {
  test("register → verify → login → first post → comment → settings → sessions", async ({
    page,
  }) => {
    const email = `e2e-${Date.now()}@test.dev`;

    // ── 1. Register through the UI, capturing the dev verification token ──
    const registerResponse = page.waitForResponse(
      (r) => r.url().includes("/api/v1/auth/register") && r.request().method() === "POST",
    );
    await page.goto("/register");
    await page.fill("#register-email", email);
    await page.fill("#register-first", "E2E");
    await page.fill("#register-last", "Smoke");
    await page.fill("#register-password", PASSWORD);
    await page.fill("#register-confirm", PASSWORD);
    await page.click('button[type="submit"]');

    const registerBody = await (await registerResponse).json();
    const verificationToken = registerBody.verification_token;
    expect(verificationToken, "register must return a verification token").toBeTruthy();

    // ── 2. Verify email through the /verify UI ──
    await page.waitForURL("**/verify");
    await page.fill("#verify-token", verificationToken);
    await page.click('button:has-text("Verify email")');
    await expect(page.getByText("Email verified")).toBeVisible({ timeout: 10_000 });

    // ── 3. Log in through the UI, capturing the access token ──
    const loginResponse = page.waitForResponse(
      (r) => r.url().includes("/api/v1/auth/login") && r.request().method() === "POST",
    );
    await page.goto("/login");
    await page.fill("#login-email", email);
    await page.fill("#login-password", PASSWORD);
    await page.click('button:has-text("Sign in")');
    await (await loginResponse).json().then((b) => {
      expect(b.access_token).toBeTruthy();
    });

    // ── 4. Header reflects the authenticated session ──
    await expect(page.getByText("Sign in").first()).toBeHidden();
    await expect(page.getByText(email).first()).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText("Sign out").first()).toBeVisible();

    // ── 5. First post (created via the API: no composer UI exists yet) ──
    const accessToken = (await (await loginResponse).json()).access_token;
    const postResponse = await page.request.post("/api/v1/posts", {
      headers: { Authorization: `Bearer ${accessToken}` },
      data: {
        kind: "article",
        title: `E2E smoke post ${Date.now()}`,
        body: "A post created by the end-to-end smoke journey.",
        summary: "smoke",
      },
    });
    expect(postResponse.status()).toBe(201);
    const postBody = await postResponse.json();
    const postId = postBody.id ?? postBody.post?.id;
    expect(postId, "created post must have an id").toBeTruthy();

    // ── 6. Read the post and comment through the UI ──
    await page.goto(`/posts/${postId}`);
    await expect(page.getByText("E2E smoke post").first()).toBeVisible({ timeout: 10_000 });
    await page.fill("#comment-body", "E2E smoke comment");
    await page.click('button:has-text("Post comment")');
    await expect(page.getByText("E2E smoke comment").first()).toBeVisible({ timeout: 10_000 });

    // ── 7. Settings hub: tabs render ──
    await page.goto("/me/settings");
    await expect(page.getByRole("tab", { name: "Profile" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Security" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "Notifications" })).toBeVisible();

    // ── 8. Active sessions: current device listed ──
    await page.goto("/me/sessions");
    await expect(page.getByText("Active sessions", { exact: false })).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByText("This device").first()).toBeVisible();
  });
});

test.describe("anonymous surface", () => {
  test("sign-out reverts the header to anonymous", async ({ page }) => {
    const email = `e2e-anon-${Date.now()}@test.dev`;

    // Register + verify + login in one API pass (UI-covered above; this test
    // isolates the header behavior).
    const reg = await page.request.post("/api/v1/auth/register", {
      data: { email, password: PASSWORD },
    });
    expect(reg.status()).toBe(201);
    const regBody = await reg.json();
    const verify = await page.request.post("/api/v1/auth/verify-email", {
      data: { token: regBody.verification_token },
    });
    expect(verify.status()).toBe(200);
    const login = await page.request.post("/api/v1/auth/login", {
      data: { email, password: PASSWORD },
    });
    expect(login.status()).toBe(200);

    // Browser context carries the refresh cookie — the UI restores the session.
    await page.goto("/");
    await expect(page.getByText(email).first()).toBeVisible({ timeout: 10_000 });
    await page.getByText("Sign out").first().click();
    await expect(page.getByText("Sign in").first()).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText("Join free").first()).toBeVisible();
  });
});

