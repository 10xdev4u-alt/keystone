import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PostComposer } from "./PostComposer";

const createPostMutate = vi.hoisted(() => vi.fn());

vi.mock("../../api/hooks", () => ({
  useCreatePost: (options?: { onSuccess?: (data: unknown) => void }) => {
    createPostMutate.mockImplementation(() => {
      options?.onSuccess?.({ id: "11111111-1111-4111-8111-111111111111" });
    });
    return { mutate: createPostMutate, isPending: false, error: null };
  },
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderComposer() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <PostComposer />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("PostComposer", () => {
  beforeEach(() => {
    createPostMutate.mockReset();
  });

  it("publishes an article with title, body, summary and tags", async () => {
    const user = userEvent.setup();
    renderComposer();

    await user.type(screen.getByLabelText("Title"), "Async Rust patterns");
    await user.type(screen.getByLabelText("Summary"), "Tokio in practice");
    await user.type(screen.getByLabelText("Body"), "Tokio is the async runtime.");
    await user.type(screen.getByLabelText("Tags (comma separated)"), "rust, tokio");

    await user.click(screen.getByRole("button", { name: "Publish" }));
    expect(createPostMutate).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "article",
        title: "Async Rust patterns",
        summary: "Tokio in practice",
        body: "Tokio is the async runtime.",
        visibility: "public",
        tags: ["rust", "tokio"],
      }),
    );
  });

  it("shows a markdown preview without publishing", async () => {
    const user = userEvent.setup();
    renderComposer();

    await user.type(screen.getByLabelText("Body"), "# Heading\nSome paragraph");
    await user.click(screen.getByRole("button", { name: "Preview" }));

    expect(screen.getByRole("heading", { level: 2, name: "Heading" })).toBeInTheDocument();
    expect(screen.getByText("Some paragraph")).toBeInTheDocument();
    expect(createPostMutate).not.toHaveBeenCalled();
  });

  it("disables publish when the body is empty", async () => {
    renderComposer();
    expect(screen.getByRole("button", { name: "Publish" })).toBeDisabled();
  });
});
