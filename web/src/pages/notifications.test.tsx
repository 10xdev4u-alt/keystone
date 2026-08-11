import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NotificationsPage } from "./NotificationsPage";

const notificationsFixture = {
  notifications: [
    {
      id: 1,
      kind: "reaction",
      actor_id: "actor-1",
      entity_type: "post",
      entity_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      payload: { text: "Alice reacted to your post" },
      created_at: new Date(Date.now() - 3_600_000).toISOString(),
      is_read: false,
    },
    {
      id: 2,
      kind: "mention",
      actor_id: "actor-2",
      entity_type: "comment",
      entity_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      payload: { title: "Bob mentioned you in a comment" },
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
      is_read: true,
    },
  ],
  unread: 1,
  read_cursor: 0,
};

const markReadMutate = vi.hoisted(() => vi.fn());

vi.mock("../api/hooks", () => ({
  useNotifications: () => ({
    data: notificationsFixture,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useMarkNotificationsRead: () => ({ mutate: markReadMutate, isPending: false }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/me/notifications"]}>
        <NotificationsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("NotificationsPage", () => {
  beforeEach(() => {
    markReadMutate.mockReset();
  });

  it("lists notifications with read/unread state and an unread count", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "Notifications" })).toBeInTheDocument();
    expect(screen.getByText("1 unread")).toBeInTheDocument();
    expect(screen.getByText("Alice reacted to your post")).toBeInTheDocument();
    expect(screen.getByText("Bob mentioned you in a comment")).toBeInTheDocument();
  });

  it("links notifications to their entity and marks them read on click", async () => {
    const user = userEvent.setup();
    renderPage();
    const link = screen.getByRole("link", { name: /Alice reacted/ });
    expect(link).toHaveAttribute("href", "/posts/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    await user.click(link);
    expect(markReadMutate).toHaveBeenCalledWith({ up_to: 1 });
  });

  it("marks all read from the header button", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("button", { name: "Mark all read" }));
    expect(markReadMutate).toHaveBeenCalledWith({ up_to: null });
  });
});
