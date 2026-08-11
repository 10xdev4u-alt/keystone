import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CommunityPage } from "./CommunityPage";

const detail = {
  community: {
    id: "11111111-1111-4111-8111-111111111111",
    name: "Rust Guild",
    slug: "rust-guild",
    description: "Systems programming discussion.",
    visibility: "public",
    created_by: "22222222-2222-4222-8222-222222222222",
    created_at: "2026-07-01T10:00:00Z",
  },
};

const members = {
  members: [
    { user_id: "22222222-2222-4222-8222-222222222222", role: "owner", joined_at: "2026-07-01T10:00:00Z" },
  ],
};

const posts = {
  posts: [
    { post_id: "33333333-3333-4333-8333-333333333333", pinned: true, added_by: "22222222-2222-4222-8222-222222222222", added_at: "2026-07-02T10:00:00Z" },
  ],
};

vi.mock("../api/hooks", () => ({
  useCommunity: vi.fn(),
  useCommunityMembers: vi.fn(),
  useCommunityPosts: vi.fn(),
  useCurrentUser: vi.fn(),
  useJoinCommunity: vi.fn(),
}));

import { useCommunity, useCommunityMembers, useCommunityPosts, useCurrentUser, useJoinCommunity } from "../api/hooks";

const mockCommunity = vi.mocked(useCommunity);
const mockMembers = vi.mocked(useCommunityMembers);
const mockPosts = vi.mocked(useCommunityPosts);
const mockMe = vi.mocked(useCurrentUser);
const mockJoin = vi.mocked(useJoinCommunity);

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/communities/rust-guild"]}>
        <Routes>
          <Route path="/communities/:slug" element={<CommunityPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("CommunityPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCommunity.mockReturnValue({ data: detail, isLoading: false, isError: false, error: null, refetch: vi.fn() } as never);
    mockMembers.mockReturnValue({ data: members, isLoading: false, isError: false, error: null, refetch: vi.fn() } as never);
    mockPosts.mockReturnValue({ data: posts, isLoading: false, isError: false, error: null, refetch: vi.fn() } as never);
    mockJoin.mockReturnValue({ isPending: false, mutateAsync: vi.fn() } as never);
  });

  it("renders community detail, members and posts", () => {
    mockMe.mockReturnValue({ data: { id: "u1" }, isLoading: false } as never);
    renderPage();
    expect(screen.getByRole("heading", { name: "Rust Guild" })).toBeTruthy();
    expect(screen.getByText("Systems programming discussion.")).toBeTruthy();
    expect(screen.getByText("owner")).toBeTruthy();
    expect(screen.getByText(/📌/)).toBeTruthy();
  });

  it("shows a join button for non-members and hides it for members", () => {
    mockMe.mockReturnValue({ data: { id: "other-user" }, isLoading: false } as never);
    renderPage();
    expect(screen.getByRole("button", { name: "Join community" })).toBeTruthy();
  });

  it("shows member state when the user already belongs", () => {
    mockMe.mockReturnValue({ data: { id: "22222222-2222-4222-8222-222222222222" }, isLoading: false } as never);
    renderPage();
    expect(screen.getByText("✓ Member")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Join community" })).toBeNull();
  });

  it("joins a community on click", async () => {
    const user = userEvent.setup();
    mockMe.mockReturnValue({ data: { id: "other-user" }, isLoading: false } as never);
    renderPage();
    await user.click(screen.getByRole("button", { name: "Join community" }));
    expect(mockJoin.mock.results[0].value.mutateAsync).toHaveBeenCalled();
  });
});
