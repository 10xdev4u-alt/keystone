import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionsPage } from "./SessionsPage";

const sessionsFixture = {
  sessions: [
    {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      created_at: new Date(Date.now() - 3_600_000).toISOString(),
      expires_at: new Date(Date.now() + 86_400_000).toISOString(),
      ip_address: "203.0.113.7",
      user_agent:
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/126.0",
      current: true,
    },
    {
      id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
      expires_at: new Date(Date.now() - 3_600_000).toISOString(),
      ip_address: "198.51.100.9",
      user_agent: "Mozilla/5.0 (Linux; Android 14) Mobile Safari/537.36",
      current: false,
    },
  ],
};

const revokeMutate = vi.hoisted(() => vi.fn());
const revokeAllMutate = vi.hoisted(() => vi.fn());

vi.mock("../api/hooks", () => ({
  useCurrentUser: () => ({ data: { id: "user-1", role: "user" }, isPending: false, error: null }),
  useSessions: () => ({ data: sessionsFixture, isLoading: false, isError: false, error: null, refetch: vi.fn() }),
  useRevokeSession: () => ({ mutate: revokeMutate, isPending: false }),
  useRevokeAllSessions: () => ({ mutate: revokeAllMutate, isPending: false }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/me/sessions"]}>
        <SessionsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("SessionsPage", () => {
  beforeEach(() => {
    revokeMutate.mockReset();
    revokeAllMutate.mockReset();
  });

  it("lists sessions with device labels and the current badge", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "Active sessions" })).toBeInTheDocument();
    expect(screen.getByText("Mac")).toBeInTheDocument();
    expect(screen.getByText("Mobile device")).toBeInTheDocument();
    expect(screen.getAllByText("This device")).toHaveLength(1);
    expect(screen.getByText(/203\.0\.113\.7/)).toBeInTheDocument();
  });

  it("revokes a single non-current session", async () => {
    const user = userEvent.setup();
    renderPage();
    const revokeButtons = screen
      .getAllByRole("button", { name: "Revoke" })
      .filter((b) => !(b as HTMLButtonElement).disabled);
    expect(revokeButtons).toHaveLength(1); // current session's revoke is disabled
    await user.click(revokeButtons[0]);
    expect(revokeMutate).toHaveBeenCalledWith(
      "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      expect.anything(),
    );
  });

  it("signs out everywhere only after confirmation", async () => {
    const user = userEvent.setup();
    renderPage();
    expect(screen.queryByText(/sign out everywhere/i)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Sign out of all devices" }));
    expect(screen.getByText(/signs out every device/i)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Yes, sign out everywhere" }));
    expect(revokeAllMutate).toHaveBeenCalledTimes(1);
  });
});
