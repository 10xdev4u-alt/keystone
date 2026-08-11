import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AdminOverviewPage } from "./AdminOverviewPage";
import { AdminModerationPage } from "./AdminModerationPage";
import { AdminUsersPage } from "./AdminUsersPage";

const reportFixture = {
  reports: [
    {
      id: "11111111-1111-4111-8111-111111111111",
      reporter_id: "22222222-2222-4222-8222-222222222222",
      entity_type: "post",
      entity_id: "33333333-3333-4333-8333-333333333333",
      reason: "Spam",
      detail: "Repeated promotional links.",
      status: "open",
      created_at: new Date(Date.now() - 3_600_000).toISOString(),
    },
  ],
  limit: 50,
  offset: 0,
};

const usersFixture = {
  users: [
    {
      id: "22222222-2222-4222-8222-222222222222",
      email: "root@example.com",
      username: "root",
      role: "super_admin",
      status: "active",
      is_verified: true,
      last_login_at: null,
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
    },
  ],
  limit: 50,
  offset: 0,
};

const resolveMutate = vi.hoisted(() => vi.fn());

vi.mock("../../api/hooks", () => ({
  useAdminStatus: () => ({
    data: { status: "ok", uptime_secs: 3661, users: 42, live_sessions: 7 },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useReportQueue: () => ({
    data: reportFixture,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useResolveReport: () => ({ mutateAsync: resolveMutate, isPending: false, error: null }),
  useAdminUsers: () => ({
    data: usersFixture,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

function renderPage(node: React.ReactNode) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("AdminOverviewPage", () => {
  it("renders instance stats", () => {
    renderPage(<AdminOverviewPage />);
    expect(screen.getByText("Registered users")).toBeTruthy();
    expect(screen.getByText("42")).toBeTruthy();
    expect(screen.getByText("Live sessions")).toBeTruthy();
    expect(screen.getByText("1h 1m")).toBeTruthy();
  });
});

describe("AdminModerationPage", () => {
  beforeEach(() => resolveMutate.mockReset());

  it("lists open reports and resolves with a note", async () => {
    renderPage(<AdminModerationPage />);
    expect(screen.getByText("Spam")).toBeTruthy();
    await userEvent.type(
      screen.getByPlaceholderText("Resolution note (optional)"),
      "Confirmed spam, hid post",
    );
    await userEvent.click(screen.getByRole("button", { name: "Resolve" }));
    expect(resolveMutate).toHaveBeenCalledWith({
      id: "11111111-1111-4111-8111-111111111111",
      resolution_note: "Confirmed spam, hid post",
    });
  });
});

describe("AdminUsersPage", () => {
  it("renders the user directory with roles", () => {
    renderPage(<AdminUsersPage />);
    expect(screen.getByText("root@example.com")).toBeTruthy();
    expect(screen.getByText("super_admin")).toBeTruthy();
  });
});
