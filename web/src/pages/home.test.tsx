import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { HomePage } from "./HomePage";

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <HomePage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const pageFixture = {
  posts: [
    {
      id: "11111111-1111-4111-8111-111111111111",
      author_id: "22222222-2222-4222-8222-222222222222",
      kind: "article",
      title: "Typed APIs change everything",
      slug: "typed-apis-change-everything",
      summary: "A summary that renders in the card.",
      visibility: "public",
      view_count: 12,
      comment_count: 3,
      reaction_count: 7,
      bookmark_count: 1,
      published_at: new Date(Date.now() - 3_600_000).toISOString(),
      created_at: new Date(Date.now() - 3_600_000).toISOString(),
    },
    {
      id: "33333333-3333-4333-8333-333333333333",
      author_id: "22222222-2222-4222-8222-222222222222",
      kind: "question",
      title: "How do you isolate test schemas?",
      slug: "how-do-you-isolate-test-schemas",
      summary: null,
      visibility: "public",
      view_count: 5,
      comment_count: 0,
      reaction_count: 2,
      bookmark_count: 0,
      published_at: new Date(Date.now() - 86_400_000).toISOString(),
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
    },
  ],
  limit: 20,
  next_cursor: null,
};

vi.mock("../api/hooks", () => ({
  usePosts: vi.fn(),
}));

import { usePosts } from "../api/hooks";

const mockUsePosts = vi.mocked(usePosts);

describe("HomePage", () => {
  beforeEach(() => {
    mockUsePosts.mockReset();
  });

  it("renders the feed with post cards", () => {
    mockUsePosts.mockReturnValue({
      data: pageFixture,
      isLoading: false,
      isError: false,
      isFetching: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("Typed APIs change everything")).toBeTruthy();
    expect(screen.getByText("How do you isolate test schemas?")).toBeTruthy();
    expect(screen.getByText("article")).toBeTruthy();
    expect(screen.getByText("question")).toBeTruthy();
    expect(screen.getByText("3")).toBeTruthy(); // comment count
  });

  it("shows the empty state when there are no posts", () => {
    mockUsePosts.mockReturnValue({
      data: { posts: [], limit: 20, next_cursor: null },
      isLoading: false,
      isError: false,
      isFetching: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("No posts yet")).toBeTruthy();
  });

  it("shows a retryable error state on failure", () => {
    mockUsePosts.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      isFetching: false,
      error: new Error("boom"),
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("Couldn't load the feed")).toBeTruthy();
    expect(screen.getByText("Try again")).toBeTruthy();
  });
});
