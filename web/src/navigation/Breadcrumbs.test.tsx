import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { expectNoViolations } from "../lib/test-utils";
import { Breadcrumbs } from "./Breadcrumbs";

function renderCrumbs(items: { label: string; path?: string }[]) {
  return render(
    <MemoryRouter>
      <Breadcrumbs items={items} />
    </MemoryRouter>,
  );
}

describe("Breadcrumbs", () => {
  it("renders the trail with the last item as the current page", () => {
    renderCrumbs([
      { label: "Home", path: "/" },
      { label: "Posts", path: "/posts" },
      { label: "Rust in 2026" },
    ]);
    expect(screen.getByRole("navigation", { name: "Breadcrumb" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Home" })).toHaveAttribute("href", "/");
    expect(screen.getByRole("link", { name: "Posts" })).toHaveAttribute("href", "/posts");
    const current = screen.getByText("Rust in 2026");
    expect(current).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("link", { name: "Rust in 2026" })).not.toBeInTheDocument();
  });

  it("renders nothing for an empty trail", () => {
    renderCrumbs([]);
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
  });

  it("passes WCAG A/AA", async () => {
    const { container } = renderCrumbs([
      { label: "Home", path: "/" },
      { label: "Settings" },
    ]);
    await expectNoViolations(container);
  });
});
