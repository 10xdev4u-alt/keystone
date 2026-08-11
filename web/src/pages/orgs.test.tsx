import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { OrgPage } from "./OrgPage";
import { OrgsPage } from "./OrgsPage";

const orgFixture = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "Wyrm Systems",
  slug: "wyrm-systems",
  description: "Rust consultancy building production systems.",
  website: "https://wyrmsystems.dev",
  industry: "Software",
  created_by: "22222222-2222-4222-8222-222222222222",
  created_at: "2026-06-01T10:00:00Z",
};

vi.mock("../api/hooks", () => ({
  useOrgs: vi.fn(),
  useOrg: vi.fn(),
}));

import { useOrg, useOrgs } from "../api/hooks";

const mockOrgs = vi.mocked(useOrgs);
const mockOrg = vi.mocked(useOrg);

function renderList() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <OrgsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

function renderDetail() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/orgs/wyrm-systems"]}>
        <Routes>
          <Route path="/orgs/:slug" element={<OrgPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("OrgsPage", () => {
  beforeEach(() => {
    mockOrgs.mockReset();
    mockOrg.mockReset();
  });

  it("renders org cards with links", () => {
    mockOrgs.mockReturnValue({
      data: { organizations: [orgFixture] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderList();
    expect(screen.getByRole("heading", { name: "Organizations" })).toBeTruthy();
    expect(screen.getByText("Wyrm Systems")).toBeTruthy();
    expect(screen.getByRole("link", { name: /Wyrm Systems/ })).toHaveAttribute(
      "href",
      "/orgs/wyrm-systems",
    );
  });

  it("shows the empty state", () => {
    mockOrgs.mockReturnValue({
      data: { organizations: [] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderList();
    expect(screen.getByText("No organizations yet")).toBeTruthy();
  });
});

describe("OrgPage", () => {
  it("renders org detail with a safe external link", () => {
    mockOrg.mockReturnValue({
      data: { organization: orgFixture },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    } as never);

    renderDetail();
    expect(screen.getByRole("heading", { name: "Wyrm Systems" })).toBeTruthy();
    const link = screen.getByRole("link", { name: "https://wyrmsystems.dev" });
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link).toHaveAttribute("target", "_blank");
  });

  it("renders the error state when the org is missing", () => {
    mockOrg.mockReturnValue({
      data: undefined,
      isLoading: false,
      isError: true,
      error: new Error("nope"),
      refetch: vi.fn(),
    } as never);

    renderDetail();
    expect(screen.getByText("Couldn't load this organization")).toBeTruthy();
  });
});
