import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PublicLayout } from "./PublicLayout";

const logoutMutate = vi.hoisted(() => vi.fn());

// Make useCurrentUser controllable per-test (query result shape: { data }).
let currentUser: unknown = undefined;
vi.mock("../api/hooks", () => ({
  useCurrentUser: () => ({ data: currentUser }),
  useLogout: () => ({ mutate: logoutMutate, isPending: false }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderLayout() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/"]}>
        <PublicLayout />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("PublicLayout auth header", () => {
  beforeEach(() => {
    currentUser = undefined;
    logoutMutate.mockReset();
  });

  it("shows Sign in and Join free when signed out", () => {
    renderLayout();
    expect(screen.getByRole("link", { name: "Sign in" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Join free" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Settings" })).toBeNull();
  });

  it("shows the user's identity instead of auth buttons when signed in", () => {
    currentUser = { id: "u1", email: "ada@example.com", username: "ada", role: "user" };
    renderLayout();
    expect(screen.queryByRole("link", { name: "Sign in" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Join free" })).toBeNull();
    expect(screen.getByText("ada")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
  });

  it("signs out from the header", async () => {
    currentUser = { id: "u1", email: "ada@example.com", username: "ada", role: "user" };
    const user = userEvent.setup();
    renderLayout();
    await user.click(screen.getByRole("button", { name: "Sign out" }));
    expect(logoutMutate).toHaveBeenCalled();
  });
});
