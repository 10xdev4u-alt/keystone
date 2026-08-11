import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { expectNoViolations } from "../../lib/test-utils";
import { Input } from "./Input";

describe("Input", () => {
  it("associates label with the control via htmlFor", () => {
    render(<Input label="Email" />);
    const input = screen.getByLabelText("Email");
    expect(input).toBeInstanceOf(HTMLInputElement);
  });

  it("surfaces errors with aria-invalid and describedby", () => {
    render(<Input label="Email" error="required" />);
    const input = screen.getByLabelText("Email");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription("required");
    expect(screen.getByRole("alert")).toHaveTextContent("required");
  });

  it("shows a hint when no error is present", () => {
    render(<Input label="Username" hint="3-20 characters" />);
    const input = screen.getByLabelText("Username");
    expect(input).toHaveAccessibleDescription("3-20 characters");
  });

  it("accepts user typing", async () => {
    const user = userEvent.setup();
    render(<Input label="Search" />);
    await user.type(screen.getByLabelText("Search"), "rust");
    expect(screen.getByLabelText("Search")).toHaveValue("rust");
  });

  it("passes WCAG A/AA", async () => {
    const { container } = render(
      <>
        <Input label="Email" hint="We never share it" />
        <Input label="Password" error="Too short" />
      </>,
    );
    await expectNoViolations(container);
  });
});
