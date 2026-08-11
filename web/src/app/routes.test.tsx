import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { expectNoViolations } from "../lib/test-utils";
import { routesConfig } from "./routes";
import { RouteErrorBoundary } from "./ErrorBoundary";

function renderAt(path: string) {
  const router = createMemoryRouter(routesConfig, { initialEntries: [path] });
  return render(<RouterProvider router={router} />);
}

describe("routing shells", () => {
  it("renders the public shell at / with the Home placeholder", async () => {
    renderAt("/");
    // Lazy module resolves asynchronously.
    expect(await screen.findByRole("heading", { name: "Home" })).toBeInTheDocument();
    expect(screen.getByRole("banner")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Primary" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Communities" })).toBeInTheDocument();
  });

  it("renders the app shell with sidebar nav under /me", async () => {
    renderAt("/me");
    expect(await screen.findByRole("heading", { name: "My feed" })).toBeInTheDocument();
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
    await screen.findByRole("heading", { name: "Home" });
    await user.click(screen.getByRole("link", { name: "Events" }));
    expect(await screen.findByRole("heading", { name: "Events" })).toBeInTheDocument();
  });

  it("shows the NotFound page for unknown routes", async () => {
    renderAt("/does-not-exist");
    expect(await screen.findByRole("heading", { name: "Page not found" })).toBeInTheDocument();
  });

  it("passes WCAG A/AA on the public shell", async () => {
    const { container } = renderAt("/");
    await screen.findByRole("heading", { name: "Home" });
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
