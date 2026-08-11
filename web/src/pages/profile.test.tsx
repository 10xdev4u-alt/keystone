import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProfilePage } from "./ProfilePage";

const USER_ID = "22222222-2222-4222-8222-222222222222";

const profileFixture = {
  profile: {
    user_id: USER_ID,
    bio: "Building the future of Rust education.",
    location: "Berlin",
    visibility: "public",
  },
  education: [
    {
      id: "33333333-3333-4333-8333-333333333333",
      school: "TU Berlin",
      degree: "M.Sc.",
      field: "Computer Science",
      start_year: 2016,
      end_year: 2020,
      description: null,
    },
  ],
  experience: [
    {
      id: "44444444-4444-4444-8444-444444444444",
      title: "Staff Engineer",
      company: "Wyrm Systems",
      organization_id: null,
      start_date: "2021-03-01",
      end_date: null,
      current: true,
      description: null,
    },
  ],
  skills: [
    { skill: "Rust", level: "expert" },
    { skill: "SQL", level: "intermediate" },
  ],
};

vi.mock("../api/hooks", () => ({
  useCurrentUser: vi.fn(),
  useProfile: vi.fn(),
  useUpdateProfile: vi.fn(() => ({
    mutate: vi.fn(),
    isPending: false,
    error: null,
  })),
}));

import { useCurrentUser, useProfile } from "../api/hooks";

const mockMe = vi.mocked(useCurrentUser);
const mockProfile = vi.mocked(useProfile);

function renderPage() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[`/users/${USER_ID}`]}>
        <Routes>
          <Route path="/users/:userId" element={<ProfilePage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("ProfilePage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockMe.mockReturnValue({ data: { id: "someone-else" }, isLoading: false } as never);
    mockProfile.mockReturnValue({ data: profileFixture, isLoading: false } as never);
  });

  it("renders bio, location and visibility badge", () => {
    renderPage();
    expect(screen.getByText("Building the future of Rust education.")).toBeInTheDocument();
    expect(screen.getByText("📍 Berlin")).toBeInTheDocument();
    expect(screen.getByText("Public")).toBeInTheDocument();
  });

  it("renders education, experience and skills sections", () => {
    renderPage();
    expect(screen.getByText("TU Berlin")).toBeInTheDocument();
    expect(screen.getByText("M.Sc. · Computer Science")).toBeInTheDocument();
    expect(screen.getByText("Staff Engineer")).toBeInTheDocument();
    expect(screen.getByText("Rust")).toBeInTheDocument();
    expect(screen.getByText("SQL")).toBeInTheDocument();
  });

  it("shows edit button only for the profile owner", () => {
    renderPage();
    expect(screen.queryByText("Edit profile")).not.toBeInTheDocument();
  });

  it("shows the editor for the owner", () => {
    mockMe.mockReturnValue({ data: { id: USER_ID }, isLoading: false } as never);
    renderPage();
    expect(screen.getByText("Edit profile")).toBeInTheDocument();
  });

  it("renders empty states when sections have no entries", () => {
    mockProfile.mockReturnValue({
      data: { ...profileFixture, education: [], experience: [], skills: [] },
      isLoading: false,
    } as never);
    renderPage();
    expect(screen.getByText("No education listed")).toBeInTheDocument();
    expect(screen.getByText("No experience listed")).toBeInTheDocument();
    expect(screen.getByText("No skills listed")).toBeInTheDocument();
  });
});
