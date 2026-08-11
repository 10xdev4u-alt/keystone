import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { expectNoViolations } from "../../lib/test-utils";
import { Tabs } from "./Tabs";

const items = [
  { value: "posts", label: "Posts", content: <p>Post content</p> },
  { value: "comments", label: "Comments", content: <p>Comment content</p> },
];

describe("Tabs", () => {
  it("switches content on trigger click", async () => {
    const user = userEvent.setup();
    render(<Tabs items={items} />);
    expect(screen.getByText("Post content")).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "Comments" }));
    expect(screen.getByText("Comment content")).toBeInTheDocument();
    expect(screen.queryByText("Post content")).not.toBeInTheDocument();
  });

  it("navigates with arrow keys", async () => {
    const user = userEvent.setup();
    render(<Tabs items={items} />);
    const posts = screen.getByRole("tab", { name: "Posts" });
    posts.focus();
    await user.keyboard("{ArrowRight}{Enter}");
    expect(screen.getByRole("tab", { name: "Comments" })).toHaveAttribute(
      "data-state",
      "active",
    );
  });

  it("passes WCAG A/AA", async () => {
    const { container } = render(<Tabs items={items} />);
    await expectNoViolations(container);
  });
});
