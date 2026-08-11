import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MessagesPage } from "./MessagesPage";

const convosFixture = {
  conversations: [
    {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      type: "direct",
      title: null,
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
      last_message_at: new Date(Date.now() - 3_600_000).toISOString(),
      last_message: "See you tomorrow",
      unread: 2,
    },
    {
      id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      type: "group",
      title: "Rust Guild",
      created_at: new Date(Date.now() - 86_400_000).toISOString(),
      last_message_at: new Date(Date.now() - 60_000).toISOString(),
      last_message: "Merged!",
      unread: 0,
    },
  ],
};

const messagesFixture = {
  messages: [
    {
      id: "m1",
      conversation_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      sender_id: "other-user",
      body: "See you tomorrow",
      sent_at: new Date(Date.now() - 3_600_000).toISOString(),
      delivered_at: null,
      read_at: null,
    },
  ],
};

const sendMutate = vi.hoisted(() => vi.fn());
const createConvMutate = vi.hoisted(() => vi.fn());

vi.mock("../api/hooks", () => ({
  useCurrentUser: () => ({ data: { id: "me-user", role: "user" }, isLoading: false, error: null }),
  useConversations: () => ({
    data: convosFixture,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useMessages: (id: string | null) => ({
    data: id ? messagesFixture : undefined,
    isLoading: false,
    isError: false,
    error: null,
  }),
  useSendMessage: () => ({ mutate: sendMutate, isPending: false }),
  useCreateConversation: () => ({ mutate: createConvMutate, isPending: false, error: null }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/me/conversations"]}>
        <MessagesPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("MessagesPage", () => {
  beforeEach(() => {
    sendMutate.mockReset();
    createConvMutate.mockReset();
  });

  it("lists conversations with preview, unread badge and empty thread state", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "Messages" })).toBeInTheDocument();
    expect(screen.getByText("Direct message")).toBeInTheDocument();
    expect(screen.getByText("Rust Guild")).toBeInTheDocument();
    expect(screen.getByText("See you tomorrow")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument(); // unread badge
    expect(screen.getByText("Select a conversation")).toBeInTheDocument();
  });

  it("opens a thread and sends a message", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("button", { name: /Direct message/ }));
    const thread = screen.getByLabelText("Thread");
    expect(within(thread).getByText("See you tomorrow")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Message"), "Sounds good!");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(sendMutate).toHaveBeenCalledWith({ body: "Sounds good!" });
  });

  it("starts a new direct conversation from a user id", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("button", { name: "New message" }));
    await user.type(screen.getByLabelText("User ID"), "some-user-id");
    await user.click(screen.getByRole("button", { name: "Start conversation" }));
    expect(createConvMutate).toHaveBeenCalledWith({ user_id: "some-user-id" });
  });
});
