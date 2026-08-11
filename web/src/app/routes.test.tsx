import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { expectNoViolations } from "../lib/test-utils";
import { routesConfig } from "./routes";
import { RouteErrorBoundary } from "./ErrorBoundary";

// The real homepage hits the network via usePosts; stub it so shell tests
// assert routing, not data fetching.
vi.mock("../api/hooks", () => ({
  useCurrentUser: vi.fn(() => ({ data: undefined, isPending: false, error: null })),
  useUnreadCount: vi.fn(() => ({ data: { unread: 0 }, isLoading: false, isError: false })),
  useNotifications: vi.fn(() => ({
    data: { notifications: [], unread: 0, read_cursor: 0 },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useMarkNotificationsRead: vi.fn(() => ({ mutate: vi.fn(), isPending: false })),
  useLogout: vi.fn(() => ({ mutate: vi.fn(), isPending: false, error: null })),
  usePosts: vi.fn(() => ({
    data: { posts: [], limit: 20, next_cursor: null },
    isLoading: false,
    isError: false,
    isFetching: false,
    error: null,
    refetch: vi.fn(),
  })),
  useCreatePost: vi.fn(() => ({ mutate: vi.fn(), isPending: false, error: null })),
  useConversations: vi.fn(() => ({
    data: { conversations: [] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useMessages: vi.fn(() => ({ data: { messages: [] }, isLoading: false, isError: false })),
  useSendMessage: vi.fn(() => ({ mutate: vi.fn(), isPending: false })),
  useCreateConversation: vi.fn(() => ({
    mutate: vi.fn(),
    isPending: false,
    error: null,
  })),
  useEvents: vi.fn(() => ({
    data: { events: [] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useEvent: vi.fn(() => ({
    data: undefined,
    isLoading: false,
    isError: true,
    error: new Error("stub"),
    refetch: vi.fn(),
  })),
  useCourses: vi.fn(() => ({
    data: { courses: [] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useCourse: vi.fn(() => ({
    data: undefined,
    isLoading: false,
    isError: true,
    error: new Error("stub"),
    refetch: vi.fn(),
  })),
  useCommunities: vi.fn(() => ({
    data: { communities: [] },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useAdminStatus: vi.fn(() => ({
    data: { status: "ok", uptime_secs: 60, users: 1, live_sessions: 0 },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useReportQueue: vi.fn(() => ({
    data: { reports: [], limit: 50, offset: 0 },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useAdminUsers: vi.fn(() => ({
    data: { users: [], limit: 50, offset: 0 },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  })),
  useResolveReport: vi.fn(() => ({ mutateAsync: vi.fn(), isPending: false, error: null })),
}));

function renderAt(path: string) {
  const router = createMemoryRouter(routesConfig, { initialEntries: [path] });
  return render(<RouterProvider router={router} />);
}

describe("routing shells", () => {
  it("renders the public shell at / with the real homepage", async () => {
    renderAt("/");
    // Lazy module resolves asynchronously.
    expect(
      await screen.findByRole("heading", { name: "Fresh from the community" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Primary" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Communities" })).toBeInTheDocument();
  });

  it("renders the app shell with sidebar nav under /me", async () => {
    renderAt("/me");
    expect(await screen.findByRole("heading", { name: "Sign in to see your feed" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Notifications" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Messages" })).toBeInTheDocument();
  });

  it("renders the admin shell with staff nav under /admin", async () => {
    renderAt("/admin/moderation");
    expect(await screen.findByRole("heading", { name: "Moderation queue" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Users" })).toBeInTheDocument();
  });

  it("navigates between routes via nav links", async () => {
    const user = userEvent.setup();
    renderAt("/");
    await screen.findByRole("heading", { name: "Fresh from the community" });
    await user.click(screen.getByRole("link", { name: "Events" }));
    expect(await screen.findByRole("heading", { name: "Events" })).toBeInTheDocument();
  });

  it("shows the NotFound page for unknown routes", async () => {
    renderAt("/does-not-exist");
    expect(await screen.findByRole("heading", { name: "Page not found" })).toBeInTheDocument();
  });

  it("passes WCAG A/AA on the public shell", async () => {
    const { container } = renderAt("/");
    await screen.findByRole("heading", { name: "Fresh from the community" });
    await expectNoViolations(container);
  });
});

describe("route error boundary", () => {
  it("renders the ErrorState instead of a blank screen", async () => {
    const throwing = [
      {
        path: "/boom",
        errorElement: <RouteErrorBoundary />,
        element: <Boom />,
      },
    ];
    const router = createMemoryRouter(throwing as never, { initialEntries: ["/boom"] });
    render(<RouterProvider router={router} />);
    expect(
      await screen.findByRole("heading", { name: "Something went wrong" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reload page" })).toBeInTheDocument();
  });
});

function Boom(): never {
  throw new Error("route blew up");
}
