import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsPage } from "./SettingsPage";

const changePasswordMutate = vi.hoisted(() => vi.fn());
const updateProfileMutate = vi.hoisted(() => vi.fn());
const updatePrefsMutate = vi.hoisted(() => vi.fn());

vi.mock("../api/hooks", () => ({
  useCurrentUser: () => ({ data: { id: "user-1", role: "user" }, isPending: false, error: null }),
  useProfile: () => ({
    data: {
      profile: {
        bio: "Building things.",
        location: "Berlin",
        visibility: "public",
      },
      education: [],
      experience: [],
      skills: [],
    },
    isLoading: false,
    isError: false,
    error: null,
  }),
  useUpdateProfile: () => ({ mutate: updateProfileMutate, isPending: false, error: null }),
  useChangePassword: () => ({ mutate: changePasswordMutate, isPending: false, error: null }),
  useNotificationPreferences: () => ({
    data: {
      preferences: { in_app: true, digest: false, email: true, muted_kinds: [] },
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useUpdateNotificationPreferences: () => ({
    mutate: updatePrefsMutate,
    isPending: false,
    error: null,
  }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/me/settings"]}>
        <SettingsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("SettingsPage", () => {
  beforeEach(() => {
    changePasswordMutate.mockReset();
    updateProfileMutate.mockReset();
    updatePrefsMutate.mockReset();
  });

  it("shows the profile tab with prefilled bio", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Profile" })).toBeInTheDocument();
    expect(screen.getByText("Building things.")).toBeInTheDocument();
  });

  it("saves profile edits", async () => {
    const user = userEvent.setup();
    renderPage();
    const bio = screen.getByPlaceholderText(/tell the community/i);
    await user.clear(bio);
    await user.type(bio, "New bio here");
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    expect(updateProfileMutate).toHaveBeenCalledWith({
      bio: "New bio here",
      location: "Berlin",
      visibility: "public",
    });
  });

  it("changes password after validating the confirmation", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("tab", { name: "Security" }));
    await user.type(screen.getByLabelText("Current password"), "old-pass");
    await user.type(screen.getByLabelText("New password"), "new-pass-123");
    await user.type(screen.getByLabelText("Confirm new password"), "different");
    expect(screen.getByText("Passwords do not match.")).toBeInTheDocument();
    await user.clear(screen.getByLabelText("Confirm new password"));
    await user.type(screen.getByLabelText("Confirm new password"), "new-pass-123");
    await user.click(screen.getByRole("button", { name: "Change password" }));
    expect(changePasswordMutate).toHaveBeenCalledWith(
      { current_password: "old-pass", new_password: "new-pass-123" },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("links to session management from security", async () => {
    renderPage();
    await userEvent.setup().click(screen.getByRole("tab", { name: "Security" }));
    expect(screen.getByRole("link", { name: "Manage sessions" })).toHaveAttribute(
      "href",
      "/me/sessions",
    );
  });

  it("toggles notification preferences", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("tab", { name: "Notifications" }));
    const digest = screen.getByRole("switch", { name: /weekly digest/i });
    expect(digest).not.toBeChecked();
    await user.click(digest);
    expect(updatePrefsMutate).toHaveBeenCalledWith({
      in_app: true,
      digest: true,
      email: true,
    });
  });
});
