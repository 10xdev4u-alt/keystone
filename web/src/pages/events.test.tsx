import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { EventsPage } from "./EventsPage";

const fixture = {
  events: [
    {
      id: "11111111-1111-4111-8111-111111111111",
      organizer_id: "22222222-2222-4222-8222-222222222222",
      title: "Rust & Ownership Workshop",
      slug: "rust-ownership-workshop",
      description: "A hands-on workshop.",
      starts_at: "2026-09-01T18:00:00Z",
      ends_at: "2026-09-01T20:00:00Z",
      capacity: 40,
      location: "Online",
      status: "published",
    },
    {
      id: "33333333-3333-4333-8333-333333333333",
      organizer_id: "22222222-2222-4222-8222-222222222222",
      title: "Postgres Internals Talk",
      slug: "postgres-internals-talk",
      description: null,
      starts_at: "2026-09-15T17:00:00Z",
      ends_at: "2026-09-15T18:00:00Z",
      capacity: null,
      location: null,
      status: "published",
    },
  ],
};

vi.mock("../api/hooks", () => ({
  useEvents: vi.fn(),
}));

import { useEvents } from "../api/hooks";

const mockUseEvents = vi.mocked(useEvents);

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <EventsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("EventsPage", () => {
  beforeEach(() => {
    mockUseEvents.mockReset();
  });

  it("renders event cards with links", () => {
    mockUseEvents.mockReturnValue({
      data: fixture,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByRole("heading", { name: "Events" })).toBeTruthy();
    expect(screen.getByText("Rust & Ownership Workshop")).toBeTruthy();
    expect(screen.getByText("Postgres Internals Talk")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Rust & Ownership Workshop/ })).toHaveAttribute(
      "href",
      "/events/rust-ownership-workshop",
    );
  });

  it("shows the empty state", () => {
    mockUseEvents.mockReturnValue({
      data: { events: [] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("No events yet")).toBeTruthy();
  });

  it("shows a retryable error state", () => {
    mockUseEvents.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      error: new Error("boom"),
      refetch: vi.fn(),
    } as never);

    renderPage();
    expect(screen.getByText("Couldn't load events")).toBeTruthy();
  });
});
