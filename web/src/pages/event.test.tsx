import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { EventPage } from "./EventPage";

const eventFixture = {
  event: {
    id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    organizer_id: "user-1",
    title: "RustConf 2026",
    slug: "rustconf-2026",
    description: "A conference for Rust engineers.",
    starts_at: new Date(Date.now() + 86_400_000).toISOString(),
    ends_at: new Date(Date.now() + 172_800_000).toISOString(),
    capacity: 100,
    location: "Berlin",
    status: "upcoming",
  },
  speakers: ["speaker-1", "speaker-2"],
  my_registration: "registered",
};

vi.mock("../api/hooks", () => ({
  useEvent: (slug: string) => ({
    data: slug ? eventFixture : undefined,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

function renderPage() {
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/events/rustconf-2026"]}>
        <Routes>
          <Route path="/events/:slug" element={<EventPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("EventPage", () => {
  it("renders event details, speakers and registration status", () => {
    renderPage();
    expect(screen.getByRole("heading", { name: "RustConf 2026" })).toBeInTheDocument();
    expect(screen.getByText("A conference for Rust engineers.")).toBeInTheDocument();
    expect(screen.getByText(/Berlin/)).toBeInTheDocument();
    expect(screen.getByText("Capacity: 100")).toBeInTheDocument();
    expect(screen.getByText("You're registered for this event.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "speaker-1" })).toHaveAttribute(
      "href",
      "/users/speaker-1",
    );
  });
});
