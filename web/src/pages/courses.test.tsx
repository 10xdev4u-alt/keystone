import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { CoursePage } from "./CoursePage";
import { CoursesPage } from "./CoursesPage";

const coursesFixture = {
  courses: [
    {
      id: "c-1",
      author_id: "user-1",
      title: "Async Rust in Production",
      slug: "async-rust",
      description: "Tokio, channels, and error handling.",
      status: "published",
      created_at: new Date().toISOString(),
    },
  ],
};

const courseDetailFixture = {
  course: coursesFixture.courses[0],
  modules: [
    {
      id: "m-1",
      position: 1,
      title: "Foundations",
      lessons: [
        { id: "l-1", position: 1, title: "Why async?", duration_seconds: 600 },
        { id: "l-2", position: 2, title: "Futures 101", duration_seconds: null },
      ],
    },
  ],
};

vi.mock("../api/hooks", () => ({
  useCourses: () => ({
    data: coursesFixture,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useCourse: (slug: string) => ({
    data: slug ? courseDetailFixture : undefined,
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});

describe("CoursesPage", () => {
  it("lists courses with links to detail", () => {
    render(
      <QueryClientProvider client={queryClient}>
        <MemoryRouter>
          <CoursesPage />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(screen.getByRole("heading", { name: "Courses" })).toBeInTheDocument();
    expect(screen.getByText("Async Rust in Production")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Async Rust in Production" })).toHaveAttribute(
      "href",
      "/courses/async-rust",
    );
  });
});

describe("CoursePage", () => {
  it("renders the module tree with lesson durations", () => {
    render(
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/courses/async-rust"]}>
          <Routes>
            <Route path="/courses/:slug" element={<CoursePage />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>,
    );
    expect(screen.getByRole("heading", { name: "Async Rust in Production" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "1. Foundations" })).toBeInTheDocument();
    expect(screen.getByText("Why async?")).toBeInTheDocument();
    expect(screen.getByText("10m")).toBeInTheDocument();
  });
});
