import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CommunitiesPage } from "./CommunitiesPage";

const fixture = {
  communities: [
    {
      id: "11111111-1111-4111-8111-111111111111",
      name: "Rust Guild",
      slug: "rust-guild",
      description: "Systems programming, ownership, borrow-checking.",
      visibility: "public",
      created_by: "22222222-2222-4222-8222-222222222222",
      created_at: "2026-07-01T10:00:00Z",
    },
    {
      id: "33333333-3333-4333-8333-333333333333",
      name: "Postgres Pros",
      slug: "postgres-pros",
      description: null,
      visibility: "private",
      created_by: "22222222-2222-4222-8222-222222222222",
      created_at: "2026-07-02T10:00:00Z",
    },
  ],
};

vi.mock("../api/hooks", () => ({
  useCommunities: vi.fn(),
}));

import { useCommunities } from "../api/hooks";

const mockUseCommunities = vi.mocked(useCommunities);

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <CommunitiesPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("CommunitiesPage", () => {
  beforeEach(() => {
    mockUseCommunities.mockReset();
  });

  it("renders community cards with links", () => {
    mockUseCommunities.mockReturnValue({
      data: fixture,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByRole("heading", { name: "Communities" })).toBeTruthy();
    expect(screen.getByText("Rust Guild")).toBeTruthy();
    expect(screen.getByText("Postgres Pros")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Rust Guild/ })).toHaveAttribute(
      "href",
      "/communities/rust-guild",
    );
  });

  it("shows the empty state", () => {
    mockUseCommunities.mockReturnValue({
      data: { communities: [] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("No communities yet")).toBeTruthy();
  });

  it("shows a retryable error state", () => {
    mockUseCommunities.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      error: new Error("boom"),
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("Couldn't load communities")).toBeTruthy();
    expect(screen.getByText("Try again")).toBeTruthy();
  });
});
