import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { expectNoViolations } from "../../lib/test-utils";
import { Button } from "./Button";

describe("Button", () => {
  it("renders with label and fires onClick", () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Save</Button>);
    const button = screen.getByRole("button", { name: "Save" });
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("is disabled while loading and shows a spinner", () => {
    render(<Button loading>Save</Button>);
    const button = screen.getByRole("button", { name: "Save" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(screen.getByTestId("button-spinner")).toBeInTheDocument();
  });

  it("honors a plain disabled prop", () => {
    render(<Button disabled>Save</Button>);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("renders as a child element via asChild (link semantics)", () => {
    render(
      <Button asChild>
        <a href="/posts">Browse posts</a>
      </Button>,
    );
    const link = screen.getByRole("link", { name: "Browse posts" });
    expect(link).toHaveClass("btn", "btn--primary", "btn--md");
    expect(link).toHaveAttribute("href", "/posts");
  });

  it("applies variant and size classes", () => {
    const { container } = render(
      <Button variant="danger" size="lg" className="extra">
        Delete
      </Button>,
    );
    const button = container.querySelector("button");
    expect(button).toHaveClass("btn", "btn--danger", "btn--lg", "extra");
  });

  it("passes WCAG A/AA when rendered standalone", async () => {
    const { container } = render(
      <>
        <Button>Save</Button>
        <Button variant="danger">Delete</Button>
        <Button variant="ghost" loading>
          Working…
        </Button>
      </>,
    );
    await expectNoViolations(container);
  });
});
