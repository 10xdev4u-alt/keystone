import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { MyFeedPage } from "./MyFeedPage";

vi.mock("../api/hooks", () => ({
  useCurrentUser: () => ({
    data: { id: "user-1", email: "me@test.dev", first_name: "Me", role: "user" },
    isLoading: false,
    error: null,
  }),
  usePosts: () => ({
    data: {
      posts: [
        {
          id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
          kind: "article",
          title: "My first post",
          summary: "A summary",
          view_count: 12,
          comment_count: 3,
          reaction_count: 5,
          published_at: new Date(Date.now() - 3_600_000).toISOString(),
          slug: "my-first-post",
          created_at: new Date(Date.now() - 3_600_000).toISOString(),
          author_id: "user-1",
        },
      ],
      limit: 20,
      next_cursor: null,
    },
    isLoading: false,
    isError: false,
    error: null,
    isFetching: false,
    refetch: vi.fn(),
  }),
  useCreatePost: () => ({ mutate: vi.fn(), isPending: false, error: null }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/me"]}>
        <MyFeedPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("MyFeedPage", () => {
  it("shows the composer and the user's own posts", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "My feed" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Write a post" })).toBeInTheDocument();
    expect(screen.getByText("My first post")).toBeInTheDocument();
    expect(screen.getByText("A summary")).toBeInTheDocument();
    expect(screen.getByText("Your posts")).toBeInTheDocument();
  });
});
