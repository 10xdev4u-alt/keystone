import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { expectNoViolations } from "../../lib/test-utils";
import { Button } from "../Button/Button";
import { EmptyState, ErrorState, Skeleton, Spinner } from "./Status";

describe("Status components", () => {
  it("skeleton is invisible to the a11y tree", () => {
    const { container } = render(<Skeleton />);
    expect(screen.getByTestId("skeleton")).toBeInTheDocument();
    expect(container.querySelector("[aria-hidden]")).toBeInTheDocument();
  });

  it("empty state renders title, description and action", () => {
    render(
      <EmptyState title="No posts yet" description="Be the first to write." action={<Button>Write a post</Button>} />,
    );
    expect(screen.getByRole("heading", { name: "No posts yet" })).toBeInTheDocument();
    expect(screen.getByText("Be the first to write.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Write a post" })).toBeInTheDocument();
  });

  it("error state retry fires the handler", () => {
    const onRetry = vi.fn();
    render(<ErrorState message="Network failed" onRetry={onRetry} />);
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("spinner exposes an accessible label", () => {
    render(<Spinner label="Loading comments" />);
    expect(screen.getByRole("status", { name: "Loading comments" })).toBeInTheDocument();
  });

  it("passes WCAG A/AA", async () => {
    const { container } = render(
      <>
        <EmptyState title="Nothing here" />
        <ErrorState message="Boom" onRetry={() => {}} />
        <Spinner />
      </>,
    );
    await expectNoViolations(container);
  });
});
