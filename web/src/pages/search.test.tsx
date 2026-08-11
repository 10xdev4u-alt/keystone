import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SearchPage } from "./SearchPage";

vi.mock("../api/hooks", () => ({
  useSearch: vi.fn(),
}));

import { useSearch } from "../api/hooks";

const mockUseSearch = vi.mocked(useSearch);

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <SearchPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("SearchPage", () => {
  beforeEach(() => {
    mockUseSearch.mockReset();
  });

  it("shows the idle state before typing", () => {
    mockUseSearch.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: false,
      isFetching: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("Type to search")).toBeTruthy();
  });

  it("renders typed results with links", async () => {
    const user = userEvent.setup();
    mockUseSearch.mockReturnValue({
      data: {
        query: "rust",
        results: [
          {
            entity_type: "post",
            entity_id: "11111111-1111-4111-8111-111111111111",
            title: "Typed APIs change everything",
            snippet: "…a summary snippet…",
            score: 0.9,
          },
          {
            entity_type: "community",
            entity_id: "rust-guild",
            title: "Rust Guild",
            snippet: "",
            score: 0.5,
          },
        ],
      },
      isLoading: false,
      isError: false,
      isFetching: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    await user.type(screen.getByLabelText("Search the platform"), "rust");
    // Debounce: wait past 300ms.
    await new Promise((r) => setTimeout(r, 400));
    expect(await screen.findByText("Typed APIs change everything")).toBeTruthy();
    expect(screen.getByText("Rust Guild")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Typed APIs change everything/ })).toHaveAttribute(
      "href",
      "/posts/11111111-1111-4111-8111-111111111111",
    );
  });

  it("shows the empty state for no results", async () => {
    const user = userEvent.setup();
    mockUseSearch.mockReturnValue({
      data: { query: "zzz", results: [] },
      isLoading: false,
      isError: false,
      isFetching: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    await user.type(screen.getByLabelText("Search the platform"), "zzz");
    await new Promise((r) => setTimeout(r, 400));
    expect(await screen.findByText(/No results for/)).toBeTruthy();
  });
});
