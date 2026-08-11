import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PostPage } from "./PostPage";

const postFixture = {
  post: {
    id: "11111111-1111-4111-8111-111111111111",
    author_id: "22222222-2222-4222-8222-222222222222",
    kind: "article",
    title: "A post worth reading",
    slug: "a-post-worth-reading",
    body: "The full body of the post.",
    body_html: "<h2>Intro</h2><p>The full body of the post.</p><h3>Details</h3><p>More.</p>",
    summary: "Short.",
    status: "published",
    visibility: "public",
    view_count: 42,
    published_at: new Date(Date.now() - 86_400_000).toISOString(),
    created_at: new Date(Date.now() - 86_400_000).toISOString(),
    updated_at: null,
  },
};

const commentsFixture = {
  comments: [
    {
      id: "33333333-3333-4333-8333-333333333333",
      post_id: "11111111-1111-4111-8111-111111111111",
      parent_id: null,
      author_id: "22222222-2222-4222-8222-222222222222",
      body: "Great point, thanks for sharing.",
      created_at: new Date(Date.now() - 3_600_000).toISOString(),
    },
  ],
};

vi.mock("../api/hooks", () => ({
  usePost: vi.fn(),
  useComments: vi.fn(),
  useCreateComment: vi.fn(),
  useCurrentUser: vi.fn(),
  useRelatedPosts: vi.fn(),
}));

import {
  useComments,
  useCreateComment,
  useCurrentUser,
  usePost,
  useRelatedPosts,
} from "../api/hooks";

const mockUsePost = vi.mocked(usePost);
const mockUseComments = vi.mocked(useComments);
const mockUseCreateComment = vi.mocked(useCreateComment);
const mockUseCurrentUser = vi.mocked(useCurrentUser);
const mockUseRelatedPosts = vi.mocked(useRelatedPosts);

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/posts/11111111-1111-4111-8111-111111111111"]}>
        <Routes>
          <Route path="/posts/:id" element={<PostPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("PostPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUsePost.mockReturnValue({
      data: postFixture,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);
    mockUseComments.mockReturnValue({
      data: commentsFixture,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);
    mockUseCreateComment.mockReturnValue({
      isPending: false,
      mutateAsync: vi.fn().mockResolvedValue({ comment: commentsFixture.comments[0] }),
    } as never);
    mockUseRelatedPosts.mockReturnValue({
      data: { posts: [] },
      isLoading: false,
    } as never);
  });

  it("renders rich text and a table of contents from headings", () => {
    mockUseCurrentUser.mockReturnValue({ data: { id: "1" }, isLoading: false } as never);
    renderPage();
    expect(screen.getByText("The full body of the post.")).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "Table of contents" })).toBeTruthy();
    expect(screen.getAllByText("Intro")).toHaveLength(2); // TOC link + heading
    expect(screen.getAllByText("Details")).toHaveLength(2);
  });

  it("renders the related reading rail", () => {
    mockUseCurrentUser.mockReturnValue({ data: { id: "1" }, isLoading: false } as never);
    mockUseRelatedPosts.mockReturnValue({
      data: {
        posts: [
          {
            id: "33333333-3333-4333-8333-333333333333",
            kind: "article",
            title: "Async in practice",
            slug: "async-in-practice",
            summary: "A follow-up on ownership.",
            published_at: null,
          },
        ],
      },
      isLoading: false,
    } as never);
    renderPage();
    expect(screen.getByRole("heading", { name: "Related reading" })).toBeTruthy();
    expect(screen.getByText("Async in practice")).toBeTruthy();
  });

  it("renders the post body and comments", () => {
    mockUseCurrentUser.mockReturnValue({ data: { id: "1" }, isLoading: false } as never);
    renderPage();
    expect(screen.getByRole("heading", { name: "A post worth reading" })).toBeTruthy();
    expect(screen.getByText("The full body of the post.")).toBeTruthy();
    expect(screen.getByText("Great point, thanks for sharing.")).toBeTruthy();
  });

  it("hides the comment form when signed out", () => {
    mockUseCurrentUser.mockReturnValue({ data: undefined, isLoading: false } as never);
    renderPage();
    expect(screen.getByText(/Sign in/)).toBeTruthy();
    expect(screen.queryByLabelText("Add a comment")).toBeNull();
  });

  it("posts a comment when signed in", async () => {
    const user = userEvent.setup();
    mockUseCurrentUser.mockReturnValue({ data: { id: "1" }, isLoading: false } as never);
    renderPage();
    const textarea = screen.getByLabelText("Add a comment");
    await user.type(textarea, "My two cents");
    await user.click(screen.getByRole("button", { name: "Post comment" }));
    expect(mockUseCreateComment.mock.results[0].value.mutateAsync).toHaveBeenCalledWith({
      body: "My two cents",
    });
  });

  it("renders the error state when the post fails", () => {
    mockUseCurrentUser.mockReturnValue({ data: undefined, isLoading: false } as never);
    mockUsePost.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      error: new Error("nope"),
      refetch: vi.fn(),
    } as never);
    renderPage();
    expect(screen.getByText("Couldn't load this post")).toBeTruthy();
    expect(screen.getByText("Try again")).toBeTruthy();
  });
});
